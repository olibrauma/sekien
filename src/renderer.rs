use anyhow::Result;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use wry::{WebView, WebViewBuilder};
use tao::{
    event::{Event, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::WindowBuilder,
};

const MERMAID_JS: &str = include_str!("../assets/mermaid.min.js");
pub const MERMAID_VERSION: &str = "11.14.0";

#[derive(Clone, Default)]
pub struct RenderConfig {
    pub font_family: Option<String>,
    pub theme: Option<String>,
    pub look: Option<String>,
}

fn build_html(config: &RenderConfig) -> String {
    let extra_config: String = [
        ("theme",      &config.theme),
        ("fontFamily", &config.font_family),
        ("look",       &config.look),
    ]
    .iter()
    .filter_map(|(k, v)| v.as_deref().map(|v| format!(
        "  {k}: {},\n",
        serde_json::to_string(v).expect("serialize config field")
    )))
    .collect();
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
<script>{mermaid}</script>
<style>
  html, body {{ background: transparent !important; }}
</style>
</head>
<body>
<script>
mermaid.initialize({{
  startOnLoad: false,
  htmlLabels: false,
{extra_config}}});

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
        extra_config = extra_config,
    )
}

/// 全ての Mermaid ブロックをレンダリングし、完了したら on_complete を呼び出してプロセスを終了する。
/// tao のイベントループは戻ってこないため、この関数も戻ってこない。
pub fn render_all<F>(blocks: Vec<String>, config: &RenderConfig, on_complete: F) -> Result<()>
where
    F: FnOnce(Vec<String>) + Send + 'static,
{
    if blocks.is_empty() {
        on_complete(vec![]);
        std::process::exit(0);
    }

    let results = Arc::new(Mutex::new(vec![String::new(); blocks.len()]));
    let event_loop = EventLoopBuilder::<String>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let results_inner = Arc::clone(&results);
    let config_inner = config.clone();
    let blocks_inner = blocks.clone();
    let mut on_complete_opt = Some(on_complete);
    
    let mut started = false;
    let mut webview: Option<WebView> = None;
    let mut _window: Option<tao::window::Window> = None;

    event_loop.run(move |event, event_loop, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::NewEvents(StartCause::Init) => {
                if _window.is_none() {
                    let mut builder = WindowBuilder::new()
                        .with_transparent(true)
                        .with_decorations(false)
                        .with_always_on_top(false);

                    #[cfg(target_os = "linux")]
                    {
                        // Linux: size 1x1 leads to GDK assertion failures.
                        // We use a small size and position it far away.
                        builder = builder
                            .with_visible(true)
                            .with_inner_size(tao::dpi::LogicalSize::new(100, 100))
                            .with_position(tao::dpi::LogicalPosition::new(-10000, -10000));
                    }

                    #[cfg(not(target_os = "linux"))]
                    {
                        builder = builder
                            .with_visible(true)
                            .with_inner_size(tao::dpi::LogicalSize::new(1, 1))
                            .with_position(tao::dpi::LogicalPosition::new(-10000, -10000));
                    }

                    let window = match builder.build(event_loop) {
                        Ok(w) => w,
                        Err(e) => {
                            eprintln!("Error: failed to create window: {e}");
                            std::process::exit(1);
                        }
                    };
                    
                    #[cfg(not(target_os = "linux"))]
                    window.set_outer_position(tao::dpi::LogicalPosition::new(-10000, -10000));

                    _window = Some(window);
                }
            }

            Event::MainEventsCleared | Event::RedrawEventsCleared if webview.is_none() && _window.is_some() => {
                let window = _window.as_ref().unwrap();
                let proxy_inner = proxy.clone();
                let wv = match WebViewBuilder::new()
                    .with_background_color((0, 0, 0, 0))
                    .with_transparent(true)
                    .with_html(build_html(&config_inner))
                    .with_ipc_handler(move |req| {
                        let _ = proxy_inner.send_event(req.into_body());
                    })
                    .build(window)
                {
                    Ok(w) => w,
                    Err(e) => {
                        eprintln!("Error: failed to create webview: {e}");
                        std::process::exit(1);
                    }
                };
                webview = Some(wv);
            }

            Event::UserEvent(msg) => {
                let Ok(parsed) = serde_json::from_str::<Value>(&msg) else { return };
                let Some(wv) = webview.as_ref() else { return };

                match parsed["type"].as_str().unwrap_or("") {
                    "ready" if !started => {
                        started = true;
                        let js = format!(
                            "renderMermaid(0, {})",
                            Value::String(blocks_inner[0].clone())
                        );
                        let _ = wv.evaluate_script(&js);
                    }
                    "svg" => {
                        let id = parsed["id"].as_u64().unwrap_or(0) as usize;
                        if let Ok(mut res) = results_inner.lock() {
                            if id < res.len() {
                                res[id] = parsed["svg"].as_str().unwrap_or("").to_string();
                            }
                        }

                        let next = id + 1;
                        if next < blocks_inner.len() {
                            let js = format!(
                                "renderMermaid({}, {})",
                                next,
                                Value::String(blocks_inner[next].clone())
                            );
                            let _ = wv.evaluate_script(&js);
                        } else {
                            if let Some(cb) = on_complete_opt.take() {
                                let res = results_inner.lock().unwrap().clone();
                                cb(res);
                            }
                            std::process::exit(0);
                        }
                    }
                    "error" => {
                        let id = parsed["id"].as_u64().unwrap_or(0) as usize;
                        let msg = parsed["error"].as_str().unwrap_or("unknown error");
                        eprintln!("Error: mermaid block {}: {}", id + 1, msg);
                        std::process::exit(1);
                    }
                    _ => {}
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(font_family: Option<&str>, theme: Option<&str>) -> RenderConfig {
        RenderConfig {
            font_family: font_family.map(|s| s.to_string()),
            theme: theme.map(|s| s.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn build_html_no_options() {
        let html = build_html(&cfg(None, None));
        assert!(!html.contains("  fontFamily:"));
        assert!(!html.contains("  theme:"));
    }

    #[test]
    fn build_html_with_font() {
        let html = build_html(&cfg(Some("Arial"), None));
        assert!(html.contains("fontFamily: \"Arial\""));
    }

    #[test]
    fn build_html_font_single_quote_is_escaped() {
        let html = build_html(&cfg(Some("'; alert('xss'); '"), None));
        assert!(html.contains("fontFamily: \"'; alert('xss'); '\""));
        assert!(!html.contains("fontFamily: '"));
    }

    #[test]
    fn build_html_font_double_quote_is_escaped() {
        let html = build_html(&cfg(Some("Font\"Name"), None));
        assert!(html.contains("fontFamily: \"Font\\\"Name\""));
    }

    #[test]
    fn build_html_font_backslash_is_escaped() {
        let html = build_html(&cfg(Some("Font\\Name"), None));
        assert!(html.contains("fontFamily: \"Font\\\\Name\""));
    }

    #[test]
    fn build_html_with_theme() {
        let html = build_html(&cfg(None, Some("dark")));
        assert!(html.contains("theme: \"dark\""));
    }
}
