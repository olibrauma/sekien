use anyhow::{Context, Result};
use serde_json::Value;
use std::sync::{Arc, Mutex};
use wry::{WebView, WebViewBuilder};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    window::{Window, WindowId},
};

const MERMAID_JS: &str = include_str!("../assets/mermaid.min.js");

fn build_html(font_family: Option<&str>, theme: Option<&str>) -> String {
    let font_family_js = match font_family {
        Some(f) => format!(
            "  fontFamily: {},\n",
            serde_json::to_string(f).expect("serialize font family")
        ),
        None => String::new(),
    };
    let theme_js = match theme {
        Some(t) => format!(
            "  theme: {},\n",
            serde_json::to_string(t).expect("serialize theme")
        ),
        None => String::new(),
    };
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
{theme_js}{font_family_js}}});

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
        theme_js = theme_js,
        font_family_js = font_family_js,
    )
}

struct App {
    proxy: EventLoopProxy<String>,
    // 収集した SVG (呼び出し元と共有)
    results: Arc<Mutex<Vec<String>>>,
    // レンダリングエラー (呼び出し元と共有)
    error: Arc<Mutex<Option<String>>>,
    // レンダリング対象のブロック
    blocks: Vec<String>,
    font_family: Option<String>,
    theme: Option<String>,
    started: bool,
    // winit/wry ハンドル (resumed 後に初期化)
    _window: Option<Window>,
    webview: Option<WebView>,
}

impl ApplicationHandler<String> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = match event_loop
            .create_window(Window::default_attributes().with_visible(false))
        {
            Ok(w) => w,
            Err(e) => {
                *self.error.lock().unwrap() = Some(format!("failed to create window: {e}"));
                event_loop.exit();
                return;
            }
        };

        let proxy = self.proxy.clone();
        let webview = match WebViewBuilder::new(&window)
            .with_html(build_html(self.font_family.as_deref(), self.theme.as_deref()))
            .with_ipc_handler(move |req: wry::http::Request<String>| {
                let _ = proxy.send_event(req.into_body());
            })
            .build()
        {
            Ok(w) => w,
            Err(e) => {
                *self.error.lock().unwrap() = Some(format!("failed to create webview: {e}"));
                event_loop.exit();
                return;
            }
        };

        self._window = Some(window);
        self.webview = Some(webview);

        event_loop.set_control_flow(ControlFlow::Wait);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if let WindowEvent::Destroyed = event {
            event_loop.exit();
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, msg: String) {
        let Ok(parsed) = serde_json::from_str::<Value>(&msg) else { return };

        let Some(webview) = self.webview.as_ref() else { return };

        match parsed["type"].as_str().unwrap_or("") {
            "ready" if !self.started => {
                self.started = true;
                let js = format!(
                    "renderMermaid(0, {})",
                    Value::String(self.blocks[0].clone())
                );
                let _ = webview.evaluate_script(&js);
            }
            "svg" => {
                let id = parsed["id"].as_u64().unwrap_or(0) as usize;
                if let Ok(mut results) = self.results.lock() {
                    if id < results.len() {
                        results[id] = parsed["svg"].as_str().unwrap_or("").to_string();
                    }
                }

                let next = id + 1;
                if next < self.blocks.len() {
                    let js = format!(
                        "renderMermaid({}, {})",
                        next,
                        Value::String(self.blocks[next].clone())
                    );
                    let _ = webview.evaluate_script(&js);
                } else {
                    event_loop.exit();
                }
            }
            "error" => {
                let id = parsed["id"].as_u64().unwrap_or(0) as usize;
                let msg = parsed["error"].as_str().unwrap_or("unknown error");
                if let Ok(mut err) = self.error.lock() {
                    *err = Some(format!("mermaid block {}: {}", id + 1, msg));
                }
                event_loop.exit();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_html_no_font_has_no_font_family() {
        let html = build_html(None, None);
        // mermaid.initialize() の設定ブロックに fontFamily: が追加されていないこと。
        // (mermaid.min.js 自体に "fontFamily" は含まれるが、設定形式 "  fontFamily:" は含まれない)
        assert!(!html.contains("  fontFamily:"));
    }

    #[test]
    fn build_html_normal_font() {
        let html = build_html(Some("Arial"), None);
        assert!(html.contains("fontFamily: \"Arial\""));
    }

    #[test]
    fn build_html_font_single_quote_is_escaped() {
        // 修正前は fontFamily: ''; alert('xss'); '' となり JS インジェクション可能だった
        let html = build_html(Some("'; alert('xss'); '"), None);
        // JSON エンコードされているのでシングルクォートは文字列内に収まる
        assert!(html.contains("fontFamily: \"'; alert('xss'); '\""));
        // 旧形式 (シングルクォート囲み) が生成されていないこと
        assert!(!html.contains("fontFamily: '"));
    }

    #[test]
    fn build_html_font_double_quote_is_escaped() {
        let html = build_html(Some("Font\"Name"), None);
        // serde_json が \" にエスケープする
        assert!(html.contains("fontFamily: \"Font\\\"Name\""));
    }

    #[test]
    fn build_html_font_backslash_is_escaped() {
        let html = build_html(Some("Font\\Name"), None);
        assert!(html.contains("fontFamily: \"Font\\\\Name\""));
    }

    #[test]
    fn build_html_with_theme() {
        let html = build_html(None, Some("dark"));
        assert!(html.contains("theme: \"dark\""));
    }

    #[test]
    fn build_html_no_theme_has_no_theme_setting() {
        let html = build_html(None, None);
        assert!(!html.contains("  theme:"));
    }
}

pub fn render_all(blocks: Vec<String>, font_family: Option<&str>, theme: Option<&str>) -> Result<Vec<String>> {
    if blocks.is_empty() {
        return Ok(vec![]);
    }

    let results = Arc::new(Mutex::new(vec![String::new(); blocks.len()]));
    let error = Arc::new(Mutex::new(None::<String>));

    let event_loop = EventLoop::<String>::with_user_event()
        .build()
        .context("failed to build event loop")?;
    let proxy = event_loop.create_proxy();

    let mut app = App {
        proxy,
        results: Arc::clone(&results),
        error: Arc::clone(&error),
        blocks,
        font_family: font_family.map(|s| s.to_string()),
        theme: theme.map(|s| s.to_string()),
        started: false,
        _window: None,
        webview: None,
    };

    event_loop.run_app(&mut app).context("event loop error")?;

    if let Some(msg) = error.lock().unwrap().take() {
        return Err(anyhow::anyhow!(msg));
    }

    let svgs = results.lock().unwrap().clone();
    Ok(svgs)
}
