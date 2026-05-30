//! Internal render module. WebView (wry) と event loop (tao) を使って Mermaid
//! コードを SVG に変換する。sekien crate 内部からのみ使う。
//!
//! ## アーキテクチャ: streaming
//!
//! sekien は cat のような長寿命プロセスとして振る舞う。stdin (またはファイル)
//! を 1 byte ずつ読みながら `\0` 区切りで block を切り出し、到着順に webview
//! へ dispatch する。
//!
//! ```text
//! [reader thread]            [event loop / main]                [WebView / mermaid.js]
//!   |                              |                                  |
//!   |-- Block (1 件) ------------->|                                  |
//!   |                              |-- WebView 起動 (初回のみ) ----->|
//!   |                              |<-- IPC: ready -------------------|
//!   |                              |-- evaluate_script: render(N) -->|
//!   |                              |<-- IPC: svg N or error N --------|
//!   |                              |-- write SVG to stdout (即 flush) /
//!   |                              |   write error to stderr          |
//!   |-- InputEnd (EOF) ----------->|                                  |
//!   |                              |-- process::exit(0)
//! ```
//!
//! block の到着が render 完了より早ければ内部 queue に積む。block 単位の
//! render 失敗で event loop は終了せず、次 block を待つ (continue-on-error)。
//! EOF を受け取り全 pending を消化したら exit 0。sekien 自身の失敗
//! (display 初期化、IPC malformed、stdout 書き込み失敗等) のみ exit 1。

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Read, Write};
use wry::{WebView, WebViewBuilder};
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget},
    window::{Window, WindowBuilder},
};

#[cfg(target_os = "linux")]
use crate::linux_display;

const MERMAID_JS: &str = include_str!("../assets/mermaid.min.js");
const HTML_TEMPLATE: &str = include_str!("../assets/render.html");

/// `mermaid.min.js` から build script (`build.rs`) が抽出したバージョン文字列。
/// 同梱 JS の差し替え時に手動同期が要らないよう実行時バイナリに焼き込む。
pub const MERMAID_VERSION: &str = env!("MERMAID_VERSION");

#[derive(Clone, Default)]
pub struct RenderConfig {
    pub font_family: Option<String>,
    pub theme: Option<String>,
    pub look: Option<String>,
    pub show_block_ids: bool,
    /// `--config` で渡された JSON オブジェクト文字列（正規化済み）。
    /// `mermaid.initialize()` に spread される。
    pub config_json: Option<String>,
}

/// event loop で扱う user event。reader thread (Block / InputEnd / InputError)
/// と webview (Ipc) が生成する。
enum LoopEvent {
    Block(String),
    InputEnd,
    InputError(String),
    Ipc(String),
}

/// webview → sekien の IPC メッセージ。`build_html` が固定する 3 種類:
///
/// - `{"type":"ready"}`: mermaid.initialize() 完了
/// - `{"type":"svg","id":N,"svg":"..."}`: block N の SVG が出来た
/// - `{"type":"error","id":N,"error":"..."}`: block N の mermaid 解析が失敗
///
/// `#[serde(tag = "type")]` で type フィールドを discriminant として読み、
/// variant ごとのフィールド欠落 / 型不一致 / 未知 type は serde が `Err` で投げる。
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum IpcMessage {
    Ready,
    Svg { id: usize, svg: String },
    Error { id: usize, error: String },
}

/// `<script>` 内に安全に埋め込めるよう `<` / `>` を Unicode エスケープする。
///
/// serde_json は `<` / `>` をエスケープしないため、`</script>` 等が含まれると
/// script タグを早期終了させて HTML を破壊できる。
/// 文字列リテラル・JSON オブジェクトいずれの埋め込みにも使う。
fn escape_for_script(s: &str) -> String {
    s.replace('<', "\\u003c").replace('>', "\\u003e")
}

fn js_string_in_html(s: &str) -> String {
    escape_for_script(&serde_json::to_string(s).expect("serialize config field"))
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
        js_string_in_html(v)
    )))
    .collect();

    // CONFIG_JSON: --config で渡された JSON を spread する。
    // 未指定時は空オブジェクト `{}` を使う (`...{}` は no-op)。
    // 個別フラグ (EXTRA_CONFIG) が後に来るので CLI フラグが config file より優先される。
    let config_json = config.config_json.as_deref()
        .map_or_else(|| "{}".to_string(), escape_for_script);

    // `assets/render.html` の各 placeholder を差し替える。
    // mermaid.min.js / extra_config / config_json はいずれも placeholder 文字列を
    // 含まないため、ナイーブな `String::replace` で衝突しない (テンプレートエンジン不要)。
    HTML_TEMPLATE
        .replace("{{MERMAID_JS}}", MERMAID_JS)
        .replace("{{CONFIG_JSON}}", &config_json)
        .replace("{{EXTRA_CONFIG}}", &extra_config)
}

fn create_window(event_loop: &EventLoopWindowTarget<LoopEvent>) -> anyhow::Result<Window> {
    // macOS/Windows は実画面に出るので画面外配置で隠す。Linux は Xvfb 内で
    // 完結するので位置は問題にならないが、1x1 だと GDK アサーションが出る
    // ため 100x100 に拡大する。
    let size = if cfg!(target_os = "linux") { 100 } else { 1 };
    let builder = WindowBuilder::new()
        .with_transparent(true)
        .with_decorations(false)
        .with_always_on_top(false)
        .with_visible(true)
        .with_inner_size(tao::dpi::LogicalSize::new(size, size));
    #[cfg(not(target_os = "linux"))]
    let builder = builder.with_position(tao::dpi::LogicalPosition::new(-10000, -10000));
    let window = builder
        .build(event_loop)
        .map_err(|e| anyhow::anyhow!("failed to create window: {e}"))?;
    #[cfg(not(target_os = "linux"))]
    window.set_outer_position(tao::dpi::LogicalPosition::new(-10000, -10000));
    Ok(window)
}

fn create_webview(
    window: &Window,
    html: String,
    proxy: EventLoopProxy<LoopEvent>,
) -> anyhow::Result<WebView> {
    WebViewBuilder::new()
        .with_background_color((0, 0, 0, 0))
        .with_transparent(true)
        .with_html(html)
        .with_ipc_handler(move |req| {
            let _ = proxy.send_event(LoopEvent::Ipc(req.into_body()));
        })
        .build(window)
        .map_err(|e| anyhow::anyhow!("failed to create webview: {e}"))
}

/// `buf` を 1 つの `Block` として emit する。UTF-8 invalid なら `InputError` を
/// emit して `false` を返す (caller が読み込みを打ち切るシグナル)。
fn emit_block<F: FnMut(LoopEvent)>(buf: &mut Vec<u8>, on_event: &mut F) -> bool {
    String::from_utf8(std::mem::take(buf))
        .map(|s| { on_event(LoopEvent::Block(s)); true })
        .unwrap_or_else(|e| { on_event(LoopEvent::InputError(format!("input is not valid UTF-8: {e}"))); false })
}

/// 入力 stream を読みつつ `\0` で分割し、block ごとに `on_event` を呼ぶ。
///
/// `\0` を見たら現バッファ (空の場合も含む) を `LoopEvent::Block` で emit する。
/// EOF 時はバッファが非空のときだけ最後の Block を emit する (末尾 `\0` を
/// 余分な空 block にしないため)。最後に必ず `InputEnd` を 1 回 emit する。
/// I/O または UTF-8 エラーは `InputError` を emit して即 return する
/// (`InputEnd` は emit されない)。
fn read_blocks<R: Read>(reader: R, mut on_event: impl FnMut(LoopEvent)) {
    let mut reader = BufReader::new(reader);
    let mut buf: Vec<u8> = Vec::new();
    loop {
        match reader.read_until(0, &mut buf) {
            Ok(0) => break,
            Ok(_) => {
                let is_nul = buf.last() == Some(&0);
                if is_nul { buf.pop(); }
                if (is_nul || !String::from_utf8_lossy(&buf).trim().is_empty())
                    && !emit_block(&mut buf, &mut on_event) { return; }
                buf.clear();
            }
            Err(e) => {
                on_event(LoopEvent::InputError(format!("failed to read input: {e}")));
                return;
            }
        }
    }
    on_event(LoopEvent::InputEnd);
}

/// webview への dispatch を gate する 3 状態の state machine。
///
/// - `NotReady`: webview がまだ mermaid.initialize() 完了前
/// - `Idle`: webview ready かつ render が走っていない
/// - `Awaiting(N)`: block N を render 中
///
/// 遷移: `NotReady` → (ready IPC) → `Idle` → (dispatch) → `Awaiting(N)`
/// → (svg/error IPC) → `Idle`。"ready なのに awaiting でない" のような
/// 不正な組み合わせを型で disallow する。
#[derive(Debug)]
enum Pipeline {
    NotReady,
    Idle,
    Awaiting(usize),
}

/// event 処理 1 件分の継続判断。`Err` は致命的失敗。
type StepResult = anyhow::Result<Continuation>;

#[derive(Debug, PartialEq, Eq)]
enum Continuation {
    /// event loop を回し続ける
    Continue,
    /// EOF まで処理完了。caller は exit 0 する
    Done,
}

/// 入出力 streaming の進行状態。dispatch の gate は [`Pipeline`] が持つ
/// (Awaiting は同時に最大 1 件、mermaid.render が単一 webview 上で並列実行
/// できないため)。
struct StreamState {
    /// 次に reader から受け取った block に割り当てる 1-origin の連番
    next_index: usize,
    /// dispatch 待ちの block: (block 番号, content)
    queue: VecDeque<(usize, String)>,
    /// reader thread から EOF を受け取ったか
    end_received: bool,
    /// stdout に SVG を既に書き出したか (block 間 separator `\0` の判定)
    wrote_any_svg: bool,
    /// stderr にエラーを既に書き出したか (block 間 separator `\0` の判定)
    wrote_any_error: bool,
    /// webview の ready 状態と render 中 block の有無
    pipeline: Pipeline,
    /// レンダリング設定
    config: RenderConfig,
}

impl StreamState {
    fn new(config: RenderConfig) -> Self {
        Self {
            next_index: 1,
            queue: VecDeque::new(),
            end_received: false,
            wrote_any_svg: false,
            wrote_any_error: false,
            pipeline: Pipeline::NotReady,
            config,
        }
    }

    /// event を 1 つ処理する。
    ///
    /// 戻り値:
    /// - `Ok(Continuation::Continue)`: event loop を回し続ける
    /// - `Ok(Continuation::Done)`: EOF まで処理完了。caller は exit 0
    /// - `Err(msg)`: sekien 自身の致命的失敗 (InputError / IPC malformed /
    ///   stdout 書き込み失敗等)。caller は msg を stderr に出して exit 1
    fn handle(&mut self, ev: LoopEvent, wv: &WebView) -> StepResult {
        match ev {
            LoopEvent::Block(content) => {
                let id = self.next_index;
                self.next_index += 1;
                self.queue.push_back((id, content));
                self.try_dispatch_next(wv)
            }
            LoopEvent::InputEnd => {
                self.end_received = true;
                self.try_dispatch_next(wv)
            }
            LoopEvent::InputError(e) => Err(anyhow::anyhow!(e)),
            LoopEvent::Ipc(msg) => self.on_ipc(&msg, wv),
        }
    }

    /// queue から次の 1 件を dispatch する (条件が揃えば)。戻り値の意味は
    /// [`StreamState::handle`] と同じ。
    fn try_dispatch_next(&mut self, wv: &WebView) -> StepResult {
        if !matches!(self.pipeline, Pipeline::Idle) {
            return Ok(Continuation::Continue);
        }
        if let Some((id, content)) = self.queue.pop_front() {
            dispatch_render(id, &content, wv)?;
            self.pipeline = Pipeline::Awaiting(id);
            return Ok(Continuation::Continue);
        }
        // queue 空、Idle (= awaiting 無し、ready)
        Ok(if self.end_received { Continuation::Done } else { Continuation::Continue })
    }

    /// pipeline が `Awaiting(id)` であることを検証する。
    fn check_awaiting(&self, id: usize, kind: &str) -> Result<()> {
        if matches!(self.pipeline, Pipeline::Awaiting(n) if n == id) {
            Ok(())
        } else {
            Err(ipc_protocol_error(&format!(
                "'{kind}' id {id} does not match pipeline state {:?}", self.pipeline
            )))
        }
    }

    /// block comment (オプション) + content + `\n` を組み立てる。
    fn format_output(&self, id: usize, content: &str) -> String {
        let prefix = if self.config.show_block_ids { format_block_comment(id) } else { String::new() };
        format!("{prefix}{content}\n")
    }

    fn on_ipc(&mut self, msg: &str, wv: &WebView) -> StepResult {
        let parsed: IpcMessage = serde_json::from_str(msg)
            .map_err(|e| ipc_protocol_error(&format!("{e} (raw: {msg})")))?;
        match parsed {
            IpcMessage::Ready => {
                self.pipeline = Pipeline::Idle;
                self.try_dispatch_next(wv)
            }
            IpcMessage::Svg { id, svg } => {
                self.check_awaiting(id, "svg")?;
                write_output(io::stdout().lock(), &self.format_output(id, &svg), self.wrote_any_svg)
                    .context("failed to write SVG to stdout")?;
                self.wrote_any_svg = true;
                self.pipeline = Pipeline::Idle;
                self.try_dispatch_next(wv)
            }
            IpcMessage::Error { id, error } => {
                self.check_awaiting(id, "error")?;
                write_output(io::stderr().lock(), &self.format_output(id, &error), self.wrote_any_error)
                    .context("failed to write error to stderr")?;
                self.wrote_any_error = true;
                self.pipeline = Pipeline::Idle;
                self.try_dispatch_next(wv)
            }
        }
    }
}

fn format_block_comment(id: usize) -> String {
    format!("<!-- {{\"id\": {id}}} -->\n")
}

fn dispatch_render(id: usize, content: &str, wv: &WebView) -> anyhow::Result<()> {
    // serde_json の string 出力は valid な JS 文字列リテラルとしてそのまま使える
    // (`"` / `\` / 制御文字 / U+2028,U+2029 を escape)。`evaluate_script` は HTML
    // parser を介さないので `</script>` の追加 escape は不要 (HTML 埋め込み経路の
    // build_html / js_string_in_html だけが対象)。
    let content_literal = serde_json::to_string(content).expect("serialize Mermaid block content");
    let js = format!("renderMermaid({id}, {content_literal})");
    wv.evaluate_script(&js)
        .map_err(|e| anyhow::anyhow!("failed to dispatch render({id}) to webview: {e}"))
}

fn write_output(mut out: impl Write, content: &str, write_separator: bool) -> io::Result<()> {
    if write_separator { out.write_all(&[0])?; }
    out.write_all(content.as_bytes())?;
    out.flush()
}

/// IPC が想定外の形だった場合の error message を整形する。
///
/// WebView から sekien への IPC protocol は本 crate が完全に制御している
/// (HTML 側のスクリプトも `build_html` で固定) ので、ここに来る場合は内部
/// バグまたは wry の互換性破壊。silent に exit 0 すると "空の SVG が出た"
/// という形でユーザーに見えて debug 困難になるため、`Err` で持ち上げて
/// caller に exit 1 させる (`run_stream` の closure 末端 1 箇所で処理)。
fn ipc_protocol_error(detail: &str) -> anyhow::Error {
    anyhow::anyhow!("malformed IPC from webview: {detail}")
}

/// 入力 stream を取り、EOF まで streaming で Mermaid → SVG 変換を行う。
///
/// 成功した SVG は stdout に `\0` 区切りで書き出す。block 単位の render
/// 失敗は stderr にエラーメッセージを出して継続する (exit 0 で終了)。
/// sekien 自身のセットアップ失敗のみ `Result::Err` (caller の `main` が
/// anyhow chain で表示して exit 1)。
pub fn run_stream<R: Read + Send + 'static>(reader: R, config: &RenderConfig) -> Result<()> {
    #[cfg(target_os = "linux")]
    linux_display::ensure_display()?;

    let event_loop = EventLoopBuilder::<LoopEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    // WebView の早期初期化 (Pre-warming):
    // 読み取りスレッドの開始やデータの到着を待たずに、メインスレッドで WebView の
    // 構築と HTML 読み込みを開始する。これにより 1 ブロック目のレンダリング開始
    // までのレイテンシを最小化する。
    let window = create_window(&event_loop).unwrap_or_else(|e| exit_fatal(e));
    let webview = create_webview(&window, build_html(config), proxy.clone())
        .unwrap_or_else(|e| exit_fatal(e));

    std::thread::spawn(move || read_blocks(reader, |ev| { let _ = proxy.send_event(ev); }));

    let mut state = StreamState::new(config.clone());
    event_loop.run(move |event, _event_loop, control_flow| {
        *control_flow = ControlFlow::Wait;
        let _keep_window_alive = &window;
        match event {
            Event::UserEvent(ev) => dispatch_or_exit(state.handle(ev, &webview)),
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}

/// `StreamState::handle` の戻り値を解釈して終端を担当する。`Continue` のみ
/// 通常 return し、`Done` で exit 0、`Err` で exit 1。
fn dispatch_or_exit(result: StepResult) {
    match result {
        Ok(Continuation::Continue) => {}
        Ok(Continuation::Done) => std::process::exit(0),
        Err(e) => exit_fatal(e),
    }
}

/// sekien 自身の致命的失敗時の唯一の exit(1) 経路。
/// `run_stream` の event loop closure 末端からだけ呼ばれる。
fn exit_fatal(e: anyhow::Error) -> ! {
    eprintln!("Error: {e:?}");
    std::process::exit(1);
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

    #[test]
    fn build_html_font_closing_script_tag_is_escaped() {
        // `</script>` を埋め込まれても script ブロックから抜けられないこと
        let html = build_html(&cfg(Some("</script><script>alert(1)//"), None));
        assert!(!html.contains("</script><script>"));
        assert!(html.contains("\\u003c/script\\u003e\\u003cscript\\u003e"));
    }

    #[test]
    fn build_html_theme_closing_script_tag_is_escaped() {
        let html = build_html(&cfg(None, Some("</script>")));
        assert!(!html.contains("theme: \"</script>\""));
        assert!(html.contains("\\u003c/script\\u003e"));
    }

    fn cfg_with_config_json(json: &str) -> RenderConfig {
        RenderConfig {
            config_json: Some(json.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn build_html_no_config_json_uses_empty_spread() {
        let html = build_html(&cfg(None, None));
        assert!(html.contains("...{}"));
    }

    #[test]
    fn build_html_with_config_json() {
        let html = build_html(&cfg_with_config_json(r#"{"flowchart":{"curve":"basis"}}"#));
        assert!(html.contains(r#"...{"flowchart":{"curve":"basis"}}"#));
    }

    #[test]
    fn build_html_config_json_closing_script_tag_is_escaped() {
        let html = build_html(&cfg_with_config_json(r#"{"fontFamily":"a</script>b"}"#));
        // config_json 内の </script> がエスケープされていること
        assert!(html.contains("\\u003c/script\\u003e"));
        // エスケープ前の文字列が config_json として埋め込まれていないこと
        assert!(!html.contains(r#""a</script>b""#));
    }

    #[test]
    fn build_html_config_json_cli_flag_comes_after_spread() {
        // spread より後に個別フラグが来ることで CLI フラグが優先されることを確認
        let config = RenderConfig {
            theme: Some("forest".to_string()),
            config_json: Some(r#"{"theme":"dark"}"#.to_string()),
            ..Default::default()
        };
        let html = build_html(&config);
        let spread_pos = html.find("...{").unwrap();
        let theme_pos  = html.find("theme: \"forest\"").unwrap();
        assert!(spread_pos < theme_pos, "spread must appear before CLI flag override");
    }

    fn parse_ipc(s: &str) -> Result<IpcMessage, serde_json::Error> {
        serde_json::from_str(s)
    }

    #[test]
    fn ipc_message_ready_ok() {
        let m = parse_ipc(r#"{"type":"ready"}"#).unwrap();
        assert!(matches!(m, IpcMessage::Ready));
    }

    #[test]
    fn ipc_message_svg_ok() {
        let m = parse_ipc(r#"{"type":"svg","id":1,"svg":"<svg/>"}"#).unwrap();
        assert!(matches!(m, IpcMessage::Svg { id: 1, ref svg } if svg == "<svg/>"));
    }

    #[test]
    fn ipc_message_error_ok() {
        let m = parse_ipc(r#"{"type":"error","id":2,"error":"Lexical error"}"#).unwrap();
        assert!(matches!(m, IpcMessage::Error { id: 2, ref error } if error == "Lexical error"));
    }

    #[test]
    fn ipc_message_unknown_type_err() {
        assert!(parse_ipc(r#"{"type":"frobnicate"}"#).is_err());
    }

    #[test]
    fn ipc_message_missing_type_err() {
        assert!(parse_ipc(r#"{"id":1,"svg":"<svg/>"}"#).is_err());
    }

    #[test]
    fn ipc_message_svg_missing_id_err() {
        assert!(parse_ipc(r#"{"type":"svg","svg":"<svg/>"}"#).is_err());
    }

    #[test]
    fn ipc_message_svg_missing_svg_err() {
        assert!(parse_ipc(r#"{"type":"svg","id":1}"#).is_err());
    }

    #[test]
    fn ipc_message_svg_non_numeric_id_err() {
        assert!(parse_ipc(r#"{"type":"svg","id":"oops","svg":"<svg/>"}"#).is_err());
    }

    #[test]
    fn ipc_message_svg_non_string_svg_err() {
        assert!(parse_ipc(r#"{"type":"svg","id":1,"svg":42}"#).is_err());
    }

    #[test]
    fn ipc_message_error_missing_field_err() {
        assert!(parse_ipc(r#"{"type":"error","id":1}"#).is_err());
    }

    #[test]
    fn ipc_message_error_non_string_err() {
        assert!(parse_ipc(r#"{"type":"error","id":1,"error":42}"#).is_err());
    }

    // ------ read_blocks ------

    fn run_reader(bytes: &[u8]) -> (Vec<String>, Option<String>, bool) {
        let mut blocks: Vec<String> = Vec::new();
        let mut error: Option<String> = None;
        let mut ended = false;
        read_blocks(std::io::Cursor::new(bytes.to_vec()), |ev| match ev {
            LoopEvent::Block(s)      => blocks.push(s),
            LoopEvent::InputEnd      => ended = true,
            LoopEvent::InputError(e) => error = Some(e),
            LoopEvent::Ipc(_)        => unreachable!("read_blocks never emits Ipc"),
        });
        (blocks, error, ended)
    }

    #[test]
    fn reader_empty_input() {
        let (blocks, err, ended) = run_reader(b"");
        assert!(blocks.is_empty());
        assert!(err.is_none());
        assert!(ended);
    }

    #[test]
    fn reader_single_block() {
        let (blocks, _, ended) = run_reader(b"graph LR\n  A --> B");
        assert_eq!(blocks, vec!["graph LR\n  A --> B"]);
        assert!(ended);
    }

    #[test]
    fn reader_two_blocks() {
        let (blocks, _, ended) = run_reader(b"m1\0m2");
        assert_eq!(blocks, vec!["m1", "m2"]);
        assert!(ended);
    }

    #[test]
    fn reader_three_blocks() {
        let (blocks, _, _) = run_reader(b"a\0b\0c");
        assert_eq!(blocks, vec!["a", "b", "c"]);
    }

    #[test]
    fn reader_trailing_null_is_dropped() {
        let (blocks, _, _) = run_reader(b"m1\0m2\0");
        assert_eq!(blocks, vec!["m1", "m2"]);
    }

    #[test]
    fn reader_double_trailing_null_yields_one_empty() {
        let (blocks, _, _) = run_reader(b"m1\0m2\0\0");
        assert_eq!(blocks, vec!["m1", "m2", ""]);
    }

    #[test]
    fn reader_invalid_utf8_stops_reader() {
        let (_, err, ended) = run_reader(&[0xff, 0xff]);
        assert!(err.is_some());
        assert!(!ended);
    }

    #[test]
    fn reader_invalid_utf8_after_separator() {
        let (blocks, err, _) = run_reader(&[b'a', 0, 0xff, 0xff]);
        assert_eq!(blocks, vec!["a"]);
        assert!(err.is_some());
    }
}
