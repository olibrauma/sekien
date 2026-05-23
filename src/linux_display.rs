//! Linux で WebView 初期化前に display backend を解決する。
//!
//! - `GDK_BACKEND=x11` を強制する。Wayland では off-screen 配置ができないため、
//!   X11 (Xwayland 経由含む) に統一する。
//! - 既存の `$DISPLAY` の有無に関わらず常に内部 Xvfb を起動して
//!   `$DISPLAY` を上書きする。Wayland セッションで Xwayland 経由になると
//!   コンポジタがウィンドウを可視位置に配置して flash するため、これを回避する。
//! - Xvfb は `-terminate` 付きで起動するので、sekien が終了して最後の client が
//!   切断した時点で自動的に終了する。
//!
//! ## Xvfb の ready 判定
//!
//! Xvfb は `-displayfd <fd>` で起動すると、X server が client を受け付け可能に
//! なったタイミングで指定 fd に display 番号を書き出す。socket file の存在だけ
//! では server が完全に ready になる前に GTK が接続を試みて失敗するため、
//! このシグナルを待つのが正しい (xvfb-run も同様の手法)。

use anyhow::{anyhow, Context, Result};
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// Display backend を解決する。`render_all` の冒頭、GTK 初期化より前に呼ぶ。
///
/// 副作用として `GDK_BACKEND=x11` を設定し、Xvfb を起動して `DISPLAY` を
/// 上書きする (既存の `DISPLAY` があっても使わない)。
pub fn ensure_display() -> Result<()> {
    std::env::set_var("GDK_BACKEND", "x11");
    // GPU 加速を無効化して libEGL 警告を抑制し、ソフトウェアレンダリングを強制する
    std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
    std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1");

    spawn_xvfb().context("failed to start Xvfb (install xvfb)")?;
    Ok(())
}

fn spawn_xvfb() -> Result<()> {
    let mut child = Command::new("Xvfb")
        .arg("-displayfd").arg("1")
        .arg("-screen").arg("0").arg("100x100x24")
        .arg("-nolisten").arg("tcp")
        .arg("-terminate")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("spawn Xvfb")?;

    let stdout = child.stdout.take().expect("piped stdout");

    // 別スレッドで stdout を読む: read_line がブロッキングなので timeout 監視のため
    let (tx, rx) = mpsc::channel::<Result<u32>>();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let res = match reader.read_line(&mut line) {
            Ok(0) => Err(anyhow!("Xvfb closed stdout before reporting display number")),
            Ok(_) => line.trim().parse::<u32>().context("parse display number from Xvfb"),
            Err(e) => Err(e.into()),
        };
        let _ = tx.send(res);
        // 残出力を drain して Xvfb が pipe full でブロックするのを防ぐ
        let mut buf = Vec::new();
        let _ = reader.read_to_end(&mut buf);
    });

    let display_num = rx.recv_timeout(Duration::from_secs(10))
        .unwrap_or_else(|_| Err(anyhow!("Xvfb did not become ready in time")))
        .inspect_err(|_| { let _ = child.kill(); })?;
    std::env::set_var("DISPLAY", format!(":{display_num}"));
    Ok(())
}
