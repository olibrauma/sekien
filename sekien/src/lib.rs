//! sekien の WebView レンダラコアライブラリ。
//!
//! OS ネイティブの WebView ([wry]) と [tao] のイベントループを使って
//! Mermaid コードを SVG に変換する。Chromium のバンドルは不要。
//!
//! ## レンダリングの流れ
//!
//! ```text
//! [Rust: render_all()]              [WebView / mermaid.js]
//!   |                                       |
//!   |-- HTML ロード + initialize() -------->|
//!   |<-- IPC: { type: "ready" } -----------|
//!   |-- evaluate_script: renderMermaid(0) ->|
//!   |<-- IPC: { type: "svg", svg: "..." } --|
//!   |-- evaluate_script: renderMermaid(1) ->|
//!   |              ...                      |
//!   |-- process::exit(0)
//! ```
//!
//! IPC メッセージは [`EventLoopProxy<String>`][tao::event_loop::EventLoopProxy] 経由で
//! `UserEvent` としてキューに入り、[`RenderState::handle`] で処理される。
//!
//! ## 終了動作の注意
//!
//! [`render_all`] は内部で [`std::process::exit`] を呼ぶため呼び出し元に戻らない。
//! tao の [`EventLoop::run`][tao::event_loop::EventLoop::run] 自体が `-> !` を返すためであり、
//! この制約は避けられない。

use anyhow::Result;
use serde_json::Value;
use wry::{WebView, WebViewBuilder};
use tao::{
    event::{Event, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget},
    window::{Window, WindowBuilder},
};

const MERMAID_JS: &str = include_str!("../assets/mermaid.min.js");

/// 同梱している mermaid.js のバージョン。
pub const MERMAID_VERSION: &str = "11.14.0";

/// Mermaid レンダリングの設定。
///
/// フィールドが `None` の場合は mermaid.js のデフォルト値が使われる。
#[derive(Clone, Default)]
pub struct RenderConfig {
    /// フォントファミリー。CSS の `font-family` 形式で指定する。
    /// 未指定時は mermaid.js のデフォルト (`"trebuchet ms", verdana, arial, sans-serif`)。
    pub font_family: Option<String>,
    /// mermaid.js のテーマ。
    /// 指定できる値: `"default"` / `"base"` / `"dark"` / `"forest"` / `"neutral"` /
    /// `"neo"` / `"neo-dark"` / `"redux"` / `"redux-dark"` / `"null"`
    pub theme: Option<String>,
    /// 図の描画スタイル。
    /// 指定できる値: `"classic"` / `"handDrawn"` / `"neo"`
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
        // JSON 文字列エンコードは JavaScript 文字列リテラルとして有効
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

fn create_window(event_loop: &EventLoopWindowTarget<String>) -> Window {
    // Linux では 1x1 が GDK のアサーションエラーを引き起こすため 100x100 にする
    let size = if cfg!(target_os = "linux") { 100 } else { 1 };
    let window = WindowBuilder::new()
        .with_transparent(true)
        .with_decorations(false)
        .with_always_on_top(false)
        .with_visible(true)
        .with_inner_size(tao::dpi::LogicalSize::new(size, size))
        .with_position(tao::dpi::LogicalPosition::new(-10000, -10000))
        .build(event_loop)
        .unwrap_or_else(|e| {
            eprintln!("Error: failed to create window: {e}");
            std::process::exit(1);
        });
    #[cfg(not(target_os = "linux"))]
    window.set_outer_position(tao::dpi::LogicalPosition::new(-10000, -10000));
    window
}

fn create_webview(window: &Window, html: String, proxy: EventLoopProxy<String>) -> WebView {
    WebViewBuilder::new()
        .with_background_color((0, 0, 0, 0))
        .with_transparent(true)
        .with_html(html)
        .with_ipc_handler(move |req| {
            let _ = proxy.send_event(req.into_body());
        })
        .build(window)
        .unwrap_or_else(|e| {
            eprintln!("Error: failed to create webview: {e}");
            std::process::exit(1);
        })
}

struct RenderState<F> {
    blocks: Vec<String>,
    results: Vec<String>,
    on_complete: Option<F>,
    started: bool,
}

impl<F: FnOnce(Vec<String>)> RenderState<F> {
    fn new(blocks: Vec<String>, on_complete: F) -> Self {
        let len = blocks.len();
        Self { blocks, results: vec![String::new(); len], on_complete: Some(on_complete), started: false }
    }

    fn handle(&mut self, msg: &str, wv: &WebView) {
        let Ok(parsed) = serde_json::from_str::<Value>(msg) else { return };

        match parsed["type"].as_str().unwrap_or("") {
            "ready" if !self.started => {
                self.started = true;
                let js = format!("renderMermaid(0, {})", Value::String(self.blocks[0].clone()));
                let _ = wv.evaluate_script(&js);
            }
            "svg" => {
                let id = parsed["id"].as_u64().unwrap_or(0) as usize;
                if id < self.results.len() {
                    self.results[id] = parsed["svg"].as_str().unwrap_or("").to_string();
                }
                let next = id + 1;
                if next < self.blocks.len() {
                    let js = format!("renderMermaid({next}, {})", Value::String(self.blocks[next].clone()));
                    let _ = wv.evaluate_script(&js);
                    return;
                }
                if let Some(cb) = self.on_complete.take() {
                    cb(std::mem::take(&mut self.results));
                }
                std::process::exit(0);
            }
            "error" => {
                let id = parsed["id"].as_u64().unwrap_or(0) as usize;
                let msg = parsed["error"].as_str().unwrap_or("unknown error");
                eprintln!("Error: mermaid block {}: {msg}", id + 1);
                std::process::exit(1);
            }
            _ => {}
        }
    }
}

/// Mermaid コードブロックをすべて SVG にレンダリングし、完了したら `on_complete` を呼び出す。
///
/// # 引数
///
/// - `blocks`: レンダリングする Mermaid コード文字列のリスト。
/// - `config`: フォント・テーマなどのレンダリング設定。
/// - `on_complete`: 全ブロックの完了時に呼ばれるコールバック。
///   引数は `blocks` と同順の SVG 文字列リスト。
///
/// # 終了動作
///
/// この関数は呼び出し元に**戻らない**。
/// 成功時は `on_complete` を呼んだ後 `process::exit(0)` で終了する。
/// エラー時は `eprintln!` でメッセージを出力して `process::exit(1)` で終了する。
///
/// `blocks` が空の場合は `on_complete(vec![])` を呼んで即座に終了する。
///
/// # 戻り値
///
/// `Result<()>` を返すのはイベントループ開始前のエラーを `?` で伝播できるようにするため。
/// 実際にはイベントループが開始すると戻らない。
pub fn render_all<F>(blocks: Vec<String>, config: &RenderConfig, on_complete: F) -> Result<()>
where
    F: FnOnce(Vec<String>) + Send + 'static,
{
    if blocks.is_empty() {
        on_complete(vec![]);
        std::process::exit(0);
    }

    let event_loop = EventLoopBuilder::<String>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let mut html = Some(build_html(config));
    let mut state = RenderState::new(blocks, on_complete);
    let mut webview: Option<WebView> = None;
    let mut owned_window: Option<Window> = None;

    event_loop.run(move |event, event_loop, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(StartCause::Init) => {
                owned_window = Some(create_window(event_loop));
            }
            Event::MainEventsCleared | Event::RedrawEventsCleared
                if webview.is_none() && owned_window.is_some() =>
            {
                webview = Some(create_webview(owned_window.as_ref().unwrap(), html.take().unwrap(), proxy.clone()));
            }
            Event::UserEvent(msg) => {
                if let Some(wv) = webview.as_ref() {
                    state.handle(&msg, wv);
                }
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
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


