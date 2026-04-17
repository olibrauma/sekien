use serde_json::Value;
use std::sync::{Arc, Mutex};
use wry::WebViewBuilder;
use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::Window,
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

pub fn render_all(blocks: Vec<String>) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    if blocks.is_empty() {
        return Ok(vec![]);
    }

    let n = blocks.len();
    let results = Arc::new(Mutex::new(vec![String::new(); n]));
    let results_ref = Arc::clone(&results);

    // IPC で受信したメッセージを一時保管するバッファ
    let ipc_buf: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let ipc_buf_ref = Arc::clone(&ipc_buf);

    let event_loop = EventLoop::new().unwrap();

    let window = event_loop.create_window(
        Window::default_attributes().with_visible(false)
    )?;

    let webview = WebViewBuilder::new(&window)
        .with_html(build_html())
        .with_ipc_handler(move |req: wry::http::Request<String>| {
            *ipc_buf_ref.lock().unwrap() = Some(req.into_body());
        })
        .build()?;

    let mut started = false;
    let mut current = 0usize;

    event_loop.run(move |event, evl| {
        // IPC メッセージをポーリング
        evl.set_control_flow(ControlFlow::Poll);

        let msg = ipc_buf.lock().unwrap().take();
        if let Some(msg) = msg {
            if let Ok(parsed) = serde_json::from_str::<Value>(&msg) {
                match parsed["type"].as_str().unwrap_or("") {
                    "ready" if !started => {
                        started = true;
                        let js = format!(
                            "renderMermaid(0, {})",
                            serde_json::to_string(&blocks[0]).unwrap()
                        );
                        let _ = webview.evaluate_script(&js);
                    }
                    "svg" => {
                        let id = parsed["id"].as_u64().unwrap_or(0) as usize;
                        results_ref.lock().unwrap()[id] =
                            parsed["svg"].as_str().unwrap_or("").to_string();

                        current = id + 1;
                        if current < n {
                            let js = format!(
                                "renderMermaid({}, {})",
                                current,
                                serde_json::to_string(&blocks[current]).unwrap()
                            );
                            let _ = webview.evaluate_script(&js);
                        } else {
                            evl.exit();
                        }
                    }
                    "error" => {
                        eprintln!(
                            "mmsvg: {}",
                            parsed["error"].as_str().unwrap_or("unknown error")
                        );
                        evl.exit();
                    }
                    _ => {}
                }
            }
        }

        if let Event::WindowEvent { event: WindowEvent::Destroyed, .. } = event {
            evl.exit();
        }
    })?;

    Ok(Arc::try_unwrap(results).unwrap().into_inner().unwrap())
}
