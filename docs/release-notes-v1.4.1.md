## ダウンロード

`FavoriteShortcut.exe` … これ1つで動きます（.NET 不要）
`FavoriteShortcut-portable.zip` … 上記 + README / LICENSE
`FavoriteShortcutRs.exe` … Rust 版。これ1つで動きます
`FavoriteShortcutRs-portable.zip` … 上記 + Rust 版の説明 / LICENSE / ライセンス表記

以前のバージョンから更新する場合は **EXE を上書きするだけ**です。登録したデータはそのまま残ります。

## v1.4.1 の変更点

- **Rust 版のメモリの使用量を減らしました** … タスクマネージャーの「メモリ」で、管理画面を表示して 約 240MB → 約 60MB、トレイに常駐しているだけなら 約 50MB になりました（作者の PC での例。GPU やドライバーによって変わります）
  - 画面の描画に、Windows 標準の DirectX 12 を優先して使うようにしました（使えない PC では、これまでどおり Vulkan / OpenGL を自動で選びます）
  - 描画で使うメモリを、速さより使う量の少なさを優先して確保するようにしました（画面の動きは変わりません）
  - 日本語フォントのファイルを丸ごと読み込まず、必要な部分だけを参照するようにしました
- Rust 版で、ランチャーを表示した瞬間に、縦に引き伸ばされた画面が一瞬見えることがあったのを直しました
- C# 版（`FavoriteShortcut.exe`）の動作に変更はありません（バージョン番号のみ）

Changes: the Rust version (`FavoriteShortcutRs.exe`) now uses much less memory: about 240 MB → about 60 MB with the manager window open, and about 50 MB when it only sits in the tray (measured on the author's PC; it depends on the GPU and driver). It now prefers DirectX 12 for drawing, allocates GPU memory favoring low usage, and reads the Japanese fonts through memory mapping instead of loading the whole files. Also fixed a stretched image that could briefly appear when the launcher opens. The C# version is unchanged except for its version number.

## 動作環境

Windows 10 (1809 以降) / Windows 11 の 64bit

初回起動時の「WindowsによってPCが保護されました」は、未署名のアプリのため出るものです。
**詳細情報** → **実行** で起動できます。

<!-- SHA256 -->

MIT License / [第三者ライセンス表記](https://github.com/YamaPiFIT/FavoriteShortcut/blob/main/THIRD-PARTY-NOTICES.md) / [Rust 版のライセンス表記](https://github.com/YamaPiFIT/FavoriteShortcut/blob/main/rust/THIRD-PARTY-LICENSES.md)
