use serde_json::Value;
use std::sync::{Arc, Mutex};
use wry::{WebView, WebViewBuilder};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

const MERMAID_JS: &str = include_str!("../assets/mermaid.min.js");

fn build_html() -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
<script>{mermaid}</script>
</head>
<body>
<script>
mermaid.initialize({{
  startOnLoad: false,
  htmlLabels: false,
  theme: 'default',
  fontFamily: 'Noto Sans JP, sans-serif'
}});

window.renderMermaid = async function(id, code) {{
  try {{
    const {{ svg }} = await mermaid.render('d' + id, code);
    window.ipc.postMessage(JSON.stringify({{ type: 'svg', id: id, svg: svg }}));
  }} catch(e) {{
    window.ipc.postMessage(JSON.stringify({{ type: 'error', id: id, error: e.message }}));
  }}
}};

window.ipc.postMessage(JSON.stringify({{ type: 'ready' }}));
</script>
</body>
</html>"#,
        mermaid = MERMAID_JS,
    )
}

struct App {
    // IPC バッファ (IPC ハンドラと共有)
    ipc_buf: Arc<Mutex<Option<String>>>,
    // 収集した SVG (呼び出し元と共有)
    results: Arc<Mutex<Vec<String>>>,
    // レンダリング対象のブロック
    blocks: Vec<String>,
    n: usize,
    current: usize,
    started: bool,
    // winit/wry ハンドル (resumed 後に初期化)
    _window: Option<Window>,
    webview: Option<WebView>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = event_loop
            .create_window(Window::default_attributes().with_visible(false))
            .unwrap();

        let ipc_ref = Arc::clone(&self.ipc_buf);
        let webview = WebViewBuilder::new(&window)
            .with_html(build_html())
            .with_ipc_handler(move |req: wry::http::Request<String>| {
                *ipc_ref.lock().unwrap() = Some(req.into_body());
            })
            .build()
            .unwrap();

        self._window = Some(window);
        self.webview = Some(webview);

        event_loop.set_control_flow(ControlFlow::Poll);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if let WindowEvent::Destroyed = event {
            event_loop.exit();
        }
    }

    // イベントキューが空になるたびに呼ばれる — IPC ポーリングをここで行う
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let msg = self.ipc_buf.lock().unwrap().take();
        let Some(msg) = msg else { return };
        let Ok(parsed) = serde_json::from_str::<Value>(&msg) else { return };

        let webview = self.webview.as_ref().unwrap();

        match parsed["type"].as_str().unwrap_or("") {
            "ready" if !self.started => {
                self.started = true;
                let js = format!(
                    "renderMermaid(0, {})",
                    serde_json::to_string(&self.blocks[0]).unwrap()
                );
                let _ = webview.evaluate_script(&js);
            }
            "svg" => {
                let id = parsed["id"].as_u64().unwrap_or(0) as usize;
                self.results.lock().unwrap()[id] =
                    parsed["svg"].as_str().unwrap_or("").to_string();

                let next = id + 1;
                if next < self.n {
                    self.current = next;
                    let js = format!(
                        "renderMermaid({}, {})",
                        next,
                        serde_json::to_string(&self.blocks[next]).unwrap()
                    );
                    let _ = webview.evaluate_script(&js);
                } else {
                    event_loop.exit();
                }
            }
            "error" => {
                eprintln!("mmsvg: {}", parsed["error"].as_str().unwrap_or("unknown error"));
                event_loop.exit();
            }
            _ => {}
        }
    }
}

pub fn render_all(blocks: Vec<String>) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    if blocks.is_empty() {
        return Ok(vec![]);
    }

    let n = blocks.len();
    let results = Arc::new(Mutex::new(vec![String::new(); n]));

    let mut app = App {
        ipc_buf: Arc::new(Mutex::new(None)),
        results: Arc::clone(&results),
        blocks,
        n,
        current: 0,
        started: false,
        _window: None,
        webview: None,
    };

    EventLoop::new()?.run_app(&mut app)?;

    let svgs = results.lock().unwrap().clone();
    Ok(svgs)
}
