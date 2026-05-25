//! sekien-api: sekien バイナリを Rust から呼ぶための ergonomic wrapper。
//!
//! [`render_blocks`] は `sekien` バイナリを spawn して stdin/stdout/stderr で通信する。
//! caller が binary path を明示的に指定する設計のため、用途に応じて使い分けられる:
//!
//! - **PATH lookup**: `"sekien"` を渡すと PATH 上から検索される。
//!   利用者が `cargo install sekien` 済みであることを期待するパターン。
//! - **明示的なパス**: 絶対パスや caller のアプリ内に bundle した binary を渡す。
//!   self-contained に配布したいアプリ向け。
//!
//! ## 使い方
//!
//! ```rust,no_run
//! use sekien_api::{render_blocks, BlockOutcome, RenderConfig};
//!
//! let outcomes = render_blocks(
//!     "sekien",
//!     vec!["graph LR\n  A --> B".to_string()],
//!     &RenderConfig::default(),
//! )?;
//! for outcome in outcomes {
//!     match outcome {
//!         BlockOutcome::Rendered(svg) => println!("{svg}"),
//!         BlockOutcome::Failed(msg)   => eprintln!("rendering failed: {msg}"),
//!     }
//! }
//! # Ok::<(), sekien_api::SekienApiError>(())
//! ```
//!
//! caller がエラー種別で挙動を変えたい例 (sekien バイナリが見つからない時の案内):
//!
//! ```rust,no_run
//! use sekien_api::{render_blocks, RenderConfig, SekienApiError};
//! use std::io::ErrorKind;
//!
//! match render_blocks("sekien", vec![], &RenderConfig::default()) {
//!     Err(SekienApiError::Spawn { source, .. }) if source.kind() == ErrorKind::NotFound => {
//!         eprintln!("sekien is not installed. run: cargo install sekien");
//!     }
//!     Err(e) => eprintln!("{e}"),
//!     Ok(_) => {}
//! }
//! ```
//!
//! ## protocol
//!
//! `render_blocks` は内部で次のように動く:
//!
//! 1. sekien binary を spawn (`RenderConfig` を CLI フラグに変換)
//! 2. sekien バイナリが読む env (`SEKIEN_FONT` / `SEKIEN_THEME` / `SEKIEN_LOOK`)
//!    を除去 (caller プロセスの shell 由来 env の漏洩防止)
//! 3. stdin に Mermaid コードを `\0` 区切りで書き込み、stdin を close
//! 4. sekien は成功した SVG だけを stdout に `\0` 区切りで出す
//! 5. sekien は失敗 block を stderr に `Error: mermaid block N: <msg>` (1 行)
//!    で出す。これを行単位で parse し、input blocks と 1:1 対応の
//!    `Vec<BlockOutcome>` を再構成する。
//!
//! sekien バイナリの protocol 仕様は DESIGN.md を参照。

use std::collections::HashMap;
use std::ffi::OsStr;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};
use thiserror::Error;

/// sekien バイナリ自身が読む環境変数のリスト。
///
/// `render_blocks` は spawn 前にこれらを `env_remove` する。caller プロセスの
/// shell 起源で設定された値が意図せず sekien に渡るのを防ぐためで、その他の
/// `SEKIEN_*` prefix 変数 (例: caller アプリが定義する `SEKIEN_BIN` 等) は
/// 除去対象にしない。
const SEKIEN_OWNED_ENV_VARS: &[&str] = &["SEKIEN_FONT", "SEKIEN_THEME", "SEKIEN_LOOK"];

/// sekien-api が返す error の型。
///
/// caller (Rust app) は変種ごとに `match` でハンドリングできる。たとえば
/// [`SekienApiError::Spawn`] の `source.kind() == ErrorKind::NotFound` を見て
/// "sekien バイナリが見つからない" を判定し、インストール案内を出す等の挙動が
/// 書ける。[`SekienApiError::ExitFailure`] は sekien プロセスが non-zero exit
/// したケースで、`stderr` に sekien バイナリ自身が出した診断メッセージを保持し、
/// Display 経由でそのまま表示できる (caller への透過伝播)。
#[derive(Debug, Error)]
pub enum SekienApiError {
    /// sekien バイナリの spawn 自体に失敗した。
    /// `source.kind() == ErrorKind::NotFound` ならバイナリが PATH 上に無い。
    #[error("failed to spawn sekien at {path:?}")]
    Spawn {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// sekien プロセスは起動したが non-zero exit で終了した。
    /// `stderr` に sekien バイナリが出した診断が入っており、`{}` 表示で
    /// そのまま見える (例: "Error: failed to create window: ...")。
    #[error("sekien exited with {status}\n{stderr}")]
    ExitFailure { status: ExitStatus, stderr: String },

    /// sekien との I/O 通信中の予期せぬ失敗 (stdin write / stdout read 等)。
    #[error("I/O error while communicating with sekien")]
    Io(#[from] io::Error),

    /// sekien の stdout または stderr が valid UTF-8 でない。
    #[error("sekien output is not valid UTF-8")]
    Utf8(#[from] std::str::Utf8Error),

    /// sekien バイナリと sekien-api の protocol contract 違反。
    /// 例: stdout の SVG 数 + stderr の失敗数が input block 数と一致しない。
    /// 通常は sekien バイナリと sekien-api のバージョン非同期が原因。
    #[error("sekien protocol violation: {0}")]
    ProtocolViolation(String),

    /// `mermaid_version` で sekien --version の出力形式が予期しないものだった。
    #[error("unexpected sekien --version output: {0:?}")]
    VersionFormat(String),
}

/// sekien-api の戻り値型。
pub type Result<T> = std::result::Result<T, SekienApiError>;

/// 1 block 分の render 結果。
///
/// [`render_blocks`] は input blocks と同順・同数の `Vec<BlockOutcome>` を返す。
/// 失敗 block は exception を起こさず `Failed` として位置を保持する設計。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockOutcome {
    /// レンダリング成功: SVG 文字列
    Rendered(String),
    /// mermaid 解析エラー等で失敗: sekien stderr から抽出したエラーメッセージ
    Failed(String),
}

/// Mermaid レンダリングの設定。
///
/// フィールドが `None` の場合は mermaid.js のデフォルト値が使われる。
#[derive(Clone, Default)]
pub struct RenderConfig {
    /// フォントファミリー。CSS の `font-family`形式で指定する。
    pub font_family: Option<String>,
    /// mermaid.js のテーマ。
    /// 指定できる値: `"default"` / `"base"` / `"dark"` / `"forest"` / `"neutral"` /
    /// `"neo"` / `"neo-dark"` / `"redux"` / `"redux-dark"` / `"null"`
    pub theme: Option<String>,
    /// 図の描画スタイル。
    /// 指定できる値: `"classic"` / `"handDrawn"` / `"neo"`
    pub look: Option<String>,
}

/// sekien バイナリに `--version` を渡して mermaid.js のバージョン文字列を取得する。
///
/// sekien の `--version` 出力は `sekien <ver> (mermaid.js <ver>)` という形式で
/// 固定されており、本関数はその末尾の `(mermaid.js <ver>)` 部分から
/// mermaid.js のバージョンだけを抽出して返す。
///
/// バージョンを構築時定数として持たないのは、source of truth は sekien バイナリに
/// 同梱されている mermaid.min.js であり、`sekien-api` 側で持つと両者が
/// ずれる可能性があるため。実行時に問い合わせることで常に一致を保証する。
pub fn mermaid_version(sekien: impl AsRef<OsStr>) -> Result<String> {
    let sekien = sekien.as_ref();
    let output = base_command(sekien)
        .arg("--version")
        .output()
        .map_err(|source| SekienApiError::Spawn { path: sekien.into(), source })?;
    if !output.status.success() {
        return Err(SekienApiError::ExitFailure {
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    let stdout = std::str::from_utf8(&output.stdout)?;
    parse_mermaid_version(stdout)
}

/// `sekien <ver> (mermaid.js <ver>)` 形式の文字列から mermaid.js のバージョンを抽出する。
///
/// テストしやすさのため `mermaid_version` から分離している。
fn parse_mermaid_version(version_output: &str) -> Result<String> {
    let trimmed = version_output.trim();
    trimmed
        .rsplit_once("(mermaid.js ")
        .and_then(|(_, suffix)| suffix.strip_suffix(')'))
        .map(str::to_string)
        .ok_or_else(|| SekienApiError::VersionFormat(trimmed.to_string()))
}

/// sekien を spawn するための基底 `Command` を作る。env hygiene のみ適用済みで、
/// CLI フラグや stdio 設定は付けない。
///
/// caller プロセスの shell 起源で設定された `SEKIEN_FONT` / `SEKIEN_THEME` /
/// `SEKIEN_LOOK` が sekien に意図せず継承されるのを防ぐ。`SEKIEN_OWNED_ENV_VARS`
/// に載っていない env (HOME / PATH / caller 独自の `SEKIEN_BIN` 等) は touch
/// しない。
///
/// `render_blocks` と `mermaid_version` の両方からこの helper を経由させる
/// ことで env hygiene の適用範囲を統一する。
fn base_command(sekien: &OsStr) -> Command {
    let mut cmd = Command::new(sekien);
    for key in SEKIEN_OWNED_ENV_VARS {
        cmd.env_remove(key);
    }
    cmd
}

/// `render_blocks` 用の `Command` を構築する (env hygiene + CLI フラグ + piped stdio)。
fn build_command(sekien: &OsStr, config: &RenderConfig) -> Command {
    let mut cmd = base_command(sekien);
    if let Some(f) = &config.font_family { cmd.args(["--font", f]); }
    if let Some(t) = &config.theme       { cmd.args(["--theme", t]); }
    if let Some(l) = &config.look        { cmd.args(["--look", l]); }
    // 成功と失敗を正確に入力順へ並べ戻すため、常にブロック ID を付与させる
    cmd.arg("--block-id");
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

/// チャンクを XML としてパースし、(ブロックID, ルート要素の文字列範囲) を抽出する。
fn extract_meta_and_body(chunk: &str) -> Option<(usize, &str)> {
    let doc = roxmltree::Document::parse(chunk.trim()).ok()?;

    // ルートの子ノードから最初のコメントを探し、その内部の XML から ID を抽出
    let meta_comment = doc.root().children().find(|n| n.is_comment())?;
    let meta_doc = roxmltree::Document::parse(meta_comment.text()?.trim()).ok()?;
    let id: usize = meta_doc.root_element().attribute("id")?.parse().ok()?;

    // ルート要素 (svg または e) の範囲を元の文字列から切り出す
    let body = &chunk.trim()[doc.root_element().range()];

    Some((id, body))
}

/// sekien の stderr テキストから構造化 XML レコードを抽出し、`{ N: msg }` map にする。
///
/// 形式: `<!-- <block id="N"/> -->\n<e><![CDATA[\n message \n]]></e>\n` (複数件は `\0` 区切り)
/// メッセージ内の `]]>` は `]]]]><![CDATA[>` にエスケープされているため、パース時に戻す。
fn parse_stderr_failures(stderr: &str) -> HashMap<usize, String> {
    stderr
        .split('\0')
        .filter(|chunk| !chunk.is_empty())
        .filter_map(|chunk| {
            let (id, body_xml) = extract_meta_and_body(chunk)?;

            // XML パーサで CDATA 部分の生テキストを取得。
            // 前後の改行 (可読性のために付与されている) を trim する。
            // パーサが分割された CDATA を自動結合するため、手動の unescape は不要。
            let doc = roxmltree::Document::parse(body_xml).ok()?;
            let msg = doc.root_element().text()?.trim().to_string();

            Some((id, msg))
        })
        .collect()
}

/// sekien の stdout テキストから構造化レコードを抽出し、`{ N: SVG }` map にする。
///
/// 形式: `<!-- <block id="N"/> -->\n<svg>...</svg>\n` (複数件は `\0` 区切り)
/// 出力された SVG から `<!-- <block id="N"/> -->` コメント行を除去して返す。
fn parse_stdout_svgs(stdout: &str) -> HashMap<usize, String> {
    stdout
        .split('\0')
        .filter(|chunk| !chunk.is_empty())
        .filter_map(|chunk| {
            let (id, svg) = extract_meta_and_body(chunk)?;
            Some((id, svg.to_string()))
        })
        .collect()
}

/// Mermaid コードブロック群を SVG にレンダリングする。
///
/// 内部で sekien binary を spawn し、stdin/stdout/stderr で通信する。
/// 返り値は `blocks` と同順・同数の `BlockOutcome` リスト。失敗 block は
/// `Failed(msg)` として位置を保持する (caller 側で per-block の fallback が
/// 書けるように)。
///
/// # 引数
///
/// - `sekien`: sekien バイナリの場所。
///   - 単純な文字列 (例: `"sekien"`) を渡すと PATH から検索される
///   - 絶対パスや相対パスを渡すと、その場所のバイナリを直接実行する
///   - `&str`, `String`, `&Path`, `PathBuf` 等いずれも受け取る
/// - `blocks`: レンダリングする Mermaid コード文字列のリスト
/// - `config`: フォント・テーマ等のレンダリング設定
///
/// # 戻り値
///
/// `Ok(Vec<BlockOutcome>)` は `blocks` と同順・同数。`blocks` が空のときは
/// sekien を spawn せず即 `Ok(vec![])` を返す。
///
/// # Errors
///
/// 各 variant の意味は [`SekienApiError`] 参照。
/// - [`SekienApiError::Spawn`]: sekien バイナリの起動失敗 (PATH 上に無い等)
/// - [`SekienApiError::ExitFailure`]: sekien プロセスが non-zero exit
///   (`stderr` に sekien バイナリの診断メッセージが入っている)
/// - [`SekienApiError::Io`] / [`SekienApiError::Utf8`]: stdio I/O の失敗
/// - [`SekienApiError::ProtocolViolation`]: sekien バイナリと sekien-api の
///   バージョン非同期等
pub fn render_blocks(
    sekien: impl AsRef<OsStr>,
    blocks: Vec<String>,
    config: &RenderConfig,
) -> Result<Vec<BlockOutcome>> {
    if blocks.is_empty() {
        return Ok(vec![]);
    }

    let sekien = sekien.as_ref();
    let mut child = build_command(sekien, config)
        .spawn()
        .map_err(|source| SekienApiError::Spawn { path: sekien.into(), source })?;

    {
        let mut stdin = child.stdin.take().expect("piped stdin");
        // 各 block の末尾に \0 を書く。末尾 1 個は sekien 側の trailing drop 規約で
        // 吸収されるので block 数のズレは生じず、空文字列を含む blocks でも
        // (例: ["m1", "", "m2"] → "m1\0\0m2\0") 意図通り N 個として解釈される。
        blocks.iter().try_for_each(|block| {
            stdin.write_all(block.as_bytes())?;
            stdin.write_all(&[0])
        })?;
    }

    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(SekienApiError::ExitFailure {
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }

    let stdout_str = std::str::from_utf8(&output.stdout)?;
    let success_svgs = parse_stdout_svgs(stdout_str);

    let stderr_str = std::str::from_utf8(&output.stderr)?;
    let failures = parse_stderr_failures(stderr_str);

    build_outcomes(blocks.len(), success_svgs, failures, stderr_str, stdout_str)
}

/// stdout 由来の SVG 列と stderr 由来の per-block 失敗 map を組み合わせて
/// input blocks と 1:1 対応의 `Vec<BlockOutcome>` を組み立てる。
fn build_outcomes(
    block_count: usize,
    success_svgs: HashMap<usize, String>,
    failures: HashMap<usize, String>,
    stderr_for_error: &str,
    stdout_for_error: &str,
) -> Result<Vec<BlockOutcome>> {
    (1..=block_count)
        .map(|n| {
            success_svgs.get(&n).map(|s| BlockOutcome::Rendered(s.clone()))
                .or_else(|| failures.get(&n).map(|m| BlockOutcome::Failed(m.clone())))
        })
        .collect::<Option<Vec<_>>>()
        .filter(|v| v.len() == success_svgs.len() + failures.len()) // 余剰データの混入がないか
        .ok_or_else(|| SekienApiError::ProtocolViolation(format!(
            "sekien output mismatch ({} SVGs, {} failures). \nstdout:\n{}\nstderr:\n{}",
            success_svgs.len(), failures.len(), stdout_for_error, stderr_for_error
        )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args_of(cmd: &Command) -> Vec<String> {
        cmd.get_args()
            .filter_map(|s| s.to_str().map(|s| s.to_string()))
            .collect()
    }

    fn envs_marked_for_removal(cmd: &Command) -> Vec<String> {
        cmd.get_envs()
            .filter(|(_, v)| v.is_none())
            .filter_map(|(k, _)| k.to_str().map(|s| s.to_string()))
            .collect()
    }

    #[test]
    fn render_blocks_empty_returns_empty() {
        // sekien binary が存在しなくても、空 input なら spawn 自体しない
        let result = render_blocks("nonexistent-binary", vec![], &RenderConfig::default()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn render_blocks_spawn_failure_returns_spawn_variant() {
        // PATH 上に存在しないバイナリで非空 input → Spawn { source: NotFound }
        let err = render_blocks(
            "definitely-nonexistent-sekien-binary-xyz",
            vec!["graph LR\n  A --> B".to_string()],
            &RenderConfig::default(),
        )
        .unwrap_err();
        match err {
            SekienApiError::Spawn { source, .. } => {
                assert_eq!(source.kind(), io::ErrorKind::NotFound);
            }
            other => panic!("expected Spawn variant, got: {other:?}"),
        }
    }

    #[test]
    fn build_command_uses_provided_binary_name() {
        let cmd = build_command(OsStr::new("sekien"), &RenderConfig::default());
        assert_eq!(cmd.get_program(), OsStr::new("sekien"));
    }

    #[test]
    fn build_command_uses_provided_absolute_path() {
        let cmd = build_command(OsStr::new("/usr/local/bin/sekien"), &RenderConfig::default());
        assert_eq!(cmd.get_program(), OsStr::new("/usr/local/bin/sekien"));
    }

    #[test]
    fn build_command_empty_config_has_no_arg_flags() {
        let cmd = build_command(OsStr::new("sekien"), &RenderConfig::default());
        assert_eq!(args_of(&cmd), vec!["--block-id"]);
    }

    #[test]
    fn build_command_font_only() {
        let cmd = build_command(OsStr::new("sekien"), &RenderConfig {
            font_family: Some("Hiragino Sans".to_string()),
            ..Default::default()
        });
        assert_eq!(args_of(&cmd), vec!["--font", "Hiragino Sans", "--block-id"]);
    }

    #[test]
    fn build_command_all_options() {
        let cmd = build_command(OsStr::new("sekien"), &RenderConfig {
            font_family: Some("Arial".to_string()),
            theme: Some("dark".to_string()),
            look: Some("handDrawn".to_string()),
        });
        assert_eq!(
            args_of(&cmd),
            vec!["--font", "Arial", "--theme", "dark", "--look", "handDrawn", "--block-id"]
        );
    }

    #[test]
    fn base_command_marks_sekien_owned_env_vars_for_removal() {
        // mermaid_version も base_command を経由するので、ここで env hygiene が
        // 効いていることを確認すれば mermaid_version 経由でも同じ保証が得られる。
        let cmd = base_command(OsStr::new("sekien"));
        let removed = envs_marked_for_removal(&cmd);
        for key in SEKIEN_OWNED_ENV_VARS {
            assert!(
                removed.iter().any(|k| k == key),
                "{key} should be marked for removal; removed = {removed:?}"
            );
        }
    }

    #[test]
    fn build_command_marks_sekien_owned_env_vars_for_removal() {
        let cmd = build_command(OsStr::new("sekien"), &RenderConfig::default());
        let removed = envs_marked_for_removal(&cmd);
        for key in SEKIEN_OWNED_ENV_VARS {
            assert!(
                removed.iter().any(|k| k == key),
                "{key} should be marked for removal; removed = {removed:?}"
            );
        }
    }

    #[test]
    fn build_command_does_not_touch_other_sekien_prefixed_env() {
        // SEKIEN_OWNED_ENV_VARS に載っていない SEKIEN_* env (caller アプリ独自の
        // SEKIEN_BIN 等) は除去対象にしないこと。バイネーム指定の意義。
        let cmd = build_command(OsStr::new("sekien"), &RenderConfig::default());
        let removed = envs_marked_for_removal(&cmd);
        for key in ["SEKIEN_BIN", "SEKIEN_BUNDLE_PATH", "SEKIEN_DEBUG"] {
            assert!(
                !removed.iter().any(|k| k == key),
                "{key} should NOT be marked for removal; removed = {removed:?}"
            );
        }
    }

    #[test]
    fn build_command_does_not_touch_unrelated_env() {
        let cmd = build_command(OsStr::new("sekien"), &RenderConfig::default());
        let removed = envs_marked_for_removal(&cmd);
        for key in ["PATH", "HOME", "LANG"] {
            assert!(
                !removed.iter().any(|k| k == key),
                "{key} should NOT be marked for removal; removed = {removed:?}"
            );
        }
    }

    #[test]
    fn parse_mermaid_version_typical() {
        assert_eq!(
            parse_mermaid_version("sekien 0.1.0 (mermaid.js 11.14.0)").unwrap(),
            "11.14.0"
        );
    }

    #[test]
    fn parse_mermaid_version_with_trailing_newline() {
        assert_eq!(
            parse_mermaid_version("sekien 0.1.0 (mermaid.js 11.14.0)\n").unwrap(),
            "11.14.0"
        );
    }

    #[test]
    fn parse_mermaid_version_pre_release_tag() {
        assert_eq!(
            parse_mermaid_version("sekien 0.1.0 (mermaid.js 12.0.0-beta.3)").unwrap(),
            "12.0.0-beta.3"
        );
    }

    #[test]
    fn parse_mermaid_version_missing_mermaid_section() {
        assert!(matches!(
            parse_mermaid_version("sekien 0.1.0"),
            Err(SekienApiError::VersionFormat(_))
        ));
    }

    #[test]
    fn parse_mermaid_version_no_closing_paren() {
        assert!(matches!(
            parse_mermaid_version("sekien 0.1.0 (mermaid.js 11.14.0"),
            Err(SekienApiError::VersionFormat(_))
        ));
    }

    #[test]
    fn parse_stdout_svgs_typical() {
        let stdout = "<!-- <block id=\"1\"/> -->\n<svg1/>\n\0<!-- <block id=\"2\"/> -->\n<svg2/>\n";
        let svgs = parse_stdout_svgs(stdout);
        assert_eq!(svgs.len(), 2);
        assert_eq!(svgs.get(&1).unwrap(), "<svg1/>");
        assert_eq!(svgs.get(&2).unwrap(), "<svg2/>");
    }

    #[test]
    fn parse_stdout_svgs_strips_comment_correctly() {
        // コメント行の後の最初の \n 以降を取得し、trim する
        let stdout = "<!-- <block id=\"99\"/> -->  \n   <svg>content</svg>   \n";
        let svgs = parse_stdout_svgs(stdout);
        assert_eq!(svgs.get(&99).unwrap(), "<svg>content</svg>");
    }

    #[test]
    fn parse_stderr_failures_extracts_block_numbers() {
        let stderr = "<!-- <block id=\"1\"/> -->\n<e><![CDATA[\nLexical error on line 2\n]]></e>\n\0\
                      <!-- <block id=\"3\"/> -->\n<e><![CDATA[\nParse error\n]]></e>\n";
        let failures = parse_stderr_failures(stderr);
        assert_eq!(failures.len(), 2);
        assert_eq!(failures.get(&1).unwrap(), "Lexical error on line 2");
        assert_eq!(failures.get(&3).unwrap(), "Parse error");
    }

    #[test]
    fn parse_stderr_failures_unescapes_cdata_end() {
        let stderr = "<!-- <block id=\"1\"/> -->\n<e><![CDATA[\nError at ]]]]><![CDATA[> line 1\n]]></e>\n";
        let failures = parse_stderr_failures(stderr);
        assert_eq!(failures.get(&1).unwrap(), "Error at ]]> line 1");
    }

    #[test]
    fn parse_stderr_failures_handles_empty() {
        assert!(parse_stderr_failures("").is_empty());
    }

    #[test]
    fn parse_stderr_failures_ignores_malformed_xml() {
        let stderr = "Warning: foo\n\
                      Error: not from sekien\n\
                      <!-- <invalid/> -->\n<e>invalid</e>\n";
        assert!(parse_stderr_failures(stderr).is_empty());
    }

    #[test]
    fn parse_stderr_failures_handles_colon_in_message() {
        let stderr = "<!-- <block id=\"2\"/> -->\n<e><![CDATA[\nParse error: unexpected token\n]]></e>\n";
        let failures = parse_stderr_failures(stderr);
        assert_eq!(failures.get(&2).unwrap(), "Parse error: unexpected token");
    }

    // ------ build_outcomes (両方向の数量・範囲 contract guard) ------

    fn svgs(items: &[(usize, &str)]) -> HashMap<usize, String> {
        items.iter().map(|(n, m)| (*n, m.to_string())).collect()
    }
    fn failures(items: &[(usize, &str)]) -> HashMap<usize, String> {
        items.iter().map(|(n, m)| (*n, m.to_string())).collect()
    }

    #[test]
    fn build_outcomes_all_rendered() {
        let out = build_outcomes(3, svgs(&[(1, "a"), (2, "b"), (3, "c")]), failures(&[]), "", "").unwrap();
        assert_eq!(out, vec![
            BlockOutcome::Rendered("a".into()),
            BlockOutcome::Rendered("b".into()),
            BlockOutcome::Rendered("c".into()),
        ]);
    }

    #[test]
    fn build_outcomes_mixed_preserves_positions() {
        let out = build_outcomes(3, svgs(&[(1, "a"), (3, "c")]), failures(&[(2, "oops")]), "", "").unwrap();
        assert_eq!(out, vec![
            BlockOutcome::Rendered("a".into()),
            BlockOutcome::Failed("oops".into()),
            BlockOutcome::Rendered("c".into()),
        ]);
    }

    #[test]
    fn build_outcomes_too_few_svgs_is_violation() {
        // 3 blocks 期待、SVG が 1 件不足 (failures 無し)
        let err = build_outcomes(3, svgs(&[(1, "a"), (2, "b")]), failures(&[]), "STDOUT", "STDERR").unwrap_err();
        assert!(matches!(err, SekienApiError::ProtocolViolation(_)), "got: {err:?}");
    }

    #[test]
    fn build_outcomes_too_many_svgs_is_violation() {
        // 2 blocks 期待だが SVG が 3 件
        let err = build_outcomes(2, svgs(&[(1, "a"), (2, "b"), (3, "c")]), failures(&[]), "STDOUT", "STDERR").unwrap_err();
        assert!(matches!(err, SekienApiError::ProtocolViolation(_)), "got: {err:?}");
    }

    #[test]
    fn build_outcomes_out_of_range_failure_key_is_violation() {
        let err = build_outcomes(3, svgs(&[(1, "a"), (2, "b")]), failures(&[(99, "x")]), "STDOUT", "STDERR").unwrap_err();
        assert!(matches!(err, SekienApiError::ProtocolViolation(_)), "got: {err:?}");
    }

    #[test]
    fn build_outcomes_empty_input() {
        let out = build_outcomes(0, svgs(&[]), failures(&[]), "", "").unwrap();
        assert!(out.is_empty());
    }

    // sekien バイナリ実体との contract 検証 (空 block 混在、順序保持、stderr parse 等) は
    // tests/e2e.rs に置いている (SEKIEN_TEST_BIN env で path を指定)
}
