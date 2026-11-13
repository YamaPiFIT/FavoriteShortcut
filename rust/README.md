# お気に入りショートカット（Rust 版）

C# 版（.NET 8 + WPF）と**同じ機能・同じ画面構成**を、Rust と [egui](https://github.com/emilk/egui) で作り直したものです。
データ（SQLite のデータベース・アイコン・バックアップ・設定）は C# 版と**まったく同じ形式**で、
同じ保存場所をそのまま使います。どちらで登録・編集しても、もう一方でそのまま表示されます。

| | C# 版 | Rust 版 |
|---|---|---|
| EXE | `FavoriteShortcut.exe` | `FavoriteShortcutRs.exe` |
| 画面 | WPF | egui（winit + wgpu） |
| データ | `Data\favorite-shortcuts.sqlite` ほか | 同じもの（共有） |
| 実行に必要なもの | 自己完結版なら不要 | 不要（EXE 単体で動く） |

---

## C# 版との関係

- **同じデータを使う**: EXE と同じ場所の `Data` フォルダ（または設定画面で変えた保存場所）を共有します。
  `FavoriteShortcutRs.exe` を `FavoriteShortcut.exe` と同じフォルダに置けば、そのまま同じデータが表示されます。
- **同時には起動しない**: 二重起動を防ぐ仕組みを共有しています。片方が動いているときにもう片方を起動すると、
  動いている方の管理画面が前に出て、後から起動した方は終了します。
  切り替えるときは、トレイアイコンのメニューの「終了」で片方を終了してから、もう片方を起動してください。
- **自動起動の登録も共有**: Windows へのサインイン時の自動起動は、設定画面で最後にオンにした方の EXE が登録されます
  （登録名は同じ `FavoriteShortcut`）。
- **データベースの版**: スキーマのバージョン（1）が同じです。将来どちらかが新しい版のデータベースを作った場合、
  古い方はデータを壊さないよう、開かずに知らせて終了します（C# 版と同じ方針）。
- **エクスポートの ZIP も同じ形式**: どちらで書き出しても、どちらでも取り込めます。
- **時刻・ID・アイコンのファイル名なども同じ規則**で保存するので、行き来しても内容は変わりません。

## 入手方法

[GitHub のリリース](https://github.com/YamaPiFIT/FavoriteShortcut/releases/latest)（v1.4.0 から）で、
`FavoriteShortcutRs.exe` または `FavoriteShortcutRs-portable.zip` をダウンロードできます。
ZIP には、この説明（`README-Rust.md`）とライセンス表記（`THIRD-PARTY-LICENSES-Rust.md`）が入っています。
C# 版の ZIP と同じフォルダに展開しても、ファイルが上書きし合わない名前にしています。

## 動作環境

- Windows 10 / 11（64 bit）
- 画面の描画に GPU を使います。Windows 標準の DirectX 12 を優先し、使えない PC では Vulkan / OpenGL を使います。
  専用の GPU が無い PC でも、Windows のソフトウェア描画が使えれば動きます。
- メモリの使用量（タスクマネージャーの「メモリ」）は、管理画面を表示して 60MB 前後、トレイに常駐しているだけなら 50MB 前後です
  （GPU のメモリは使う量の少なさを優先し、日本語フォントのファイルは丸ごと読み込まずにメモリマップで参照しています）。

## 使い方

C# 版と同じです。リポジトリ直下の [README.md](../README.md) の「使い方」を参照してください。

## ビルド方法

### 必要なもの

- [Rust](https://www.rust-lang.org/tools/install)（stable。edition 2024 に対応したもの）
- Visual Studio Build Tools（C++ によるデスクトップ開発。MSVC のリンカーと Windows SDK）

SQLite はソースから一緒にビルドされるため、別途のインストールは不要です。

### 開発ビルドと実行

```powershell
cd rust
cargo run
```

### 配布用ビルド

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build-rust.ps1
```

テスト（`cargo test`）と最適化ビルドを行い、`build\rust\FavoriteShortcutRs.exe` と
`build\FavoriteShortcutRs-portable.zip` を作ります（EXE 単体で動きます）。
`rust` フォルダで `cargo build --release` を実行しても、`rust\target\release\FavoriteShortcutRs.exe` ができます。
C# 版と同じデータを使う場合は、この EXE を `FavoriteShortcut.exe` と同じフォルダに置いてください。

リリースの作成（タグを push したとき）では、GitHub Actions が同じスクリプトで Rust 版も作り、
C# 版と一緒にリリースへ添付します（[docs/RELEASE.md](../docs/RELEASE.md)）。

### テスト

```powershell
cd rust
cargo test
```

データベース・検索・設定・URL / パスの判定・ドラッグ＆ドロップの受け取りなどを確かめます。
一時フォルダだけを使うため、実データには影響しません。

C# 版で作ったデータベースを Rust 版で読み書きできるかの確認（実データのコピーで行う）:

```powershell
$env:FSC_COMPAT_DB = "C:\path\to\copy\favorite-shortcuts.sqlite"   # 必ずコピーを指定する
cargo test -- --ignored compat
```

## 開発・確認用の環境変数

| 変数 | 内容 |
|---|---|
| `FAVORITESHORTCUT_DATA_DIR` | データの保存場所（C# 版と共通）。指定すると二重起動防止の名前も別になるので、普段使いのアプリと同時に動かせます。このとき、自動起動の登録と保存場所の設定ファイルには触れず、グローバルホットキーも登録しません。 |
| `FAVORITESHORTCUT_DEV_HOTKEYS=1` | 上記のときもホットキーを登録する |
| `FAVORITESHORTCUT_DEV_SHOW` | 起動直後に開く画面（`launcher` `settings` `bookmarks` `tags` `new` `edit` `move` `folderdelete` `text` をカンマ区切り）。見た目の確認用 |
| `FAVORITESHORTCUT_DEV_QUIET=1` | ウィンドウを前面に出さない（確認中に、別のアプリで打っている文字を奪わないため） |
| `WGPU_BACKEND` | 描画方式を指定する（`dx12` / `vulkan` / `gl`）。描画がおかしいときの切り分け用。こちらは常に有効です |

`WGPU_BACKEND` 以外は、`FAVORITESHORTCUT_DATA_DIR` を指定したときだけ有効です（普段使いの動作には影響しません）。

## フォルダ構成

```
rust/
├── Cargo.toml
├── build.rs                    EXE にアイコンとバージョン情報を埋め込む
├── assets/app.ico              アプリアイコン（C# 版と同じ）
├── tools/
│   ├── gen_strings.py          C# 版の Strings.cs から対訳表（strings_table.rs）を作る
│   └── gen_licenses.py         THIRD-PARTY-LICENSES.md を作る
└── src/
    ├── main.rs                 起動処理（二重起動の防止・DB を開く）
    ├── app.rs                  アプリ全体の流れ（ウィンドウ・トレイ・ホットキー・ダイアログ）
    ├── ui/                     画面（管理画面・ランチャー・各ダイアログ・タグ入力・見た目）
    ├── db.rs / store.rs        SQLite（C# 版と同じスキーマ）と、メモリ上のデータ
    ├── settings.rs             設定（C# 版と同じキー）
    ├── search.rs / normalize.rs 検索（C# 版と同じ点数の付け方）
    ├── target.rs / launch.rs   URL / パスの判定と、開く処理（画面とは別のスレッドで行う）
    ├── icons.rs / shell_icons.rs / favicon_cache.rs  アイコン（ブラウザのキャッシュ・シェル）
    ├── bookmarks.rs            ブラウザのお気に入りの読み取り
    ├── transfer.rs             エクスポート / インポート・バックアップ
    ├── platform.rs / tray.rs / dragdrop.rs  Windows 固有の処理（ホットキー・クリップボード・トレイなど）
    ├── loc.rs / strings_table.rs  表示言語（日本語 / 英語）
    └── paths.rs / log.rs / util.rs
```

画面の文言は C# 版の `Strings.cs` から生成しています。文言を変えたときは C# 版を直してから
`python tools\gen_strings.py` で作り直してください（両方の版で同じ表示になります）。

## C# 版と違うところ

機能と画面の構成は同じですが、描画の仕組みが違うため、次の点は少し異なります。

- 文字の描画（にじみ方・太さ）やスクロールの感触、コンボボックスなどの見た目の細部
- ツールチップやメニューの出方
- フォルダツリーへ URL などの文字列をドラッグした場合、カーソルの表示は「コピー」になりますが、
  C# 版と同じくツリーには登録しません（一覧の上にドロップすると登録画面が開きます）

## ライセンス

本体は [MIT License](../LICENSE) です。組み込んでいるライブラリのライセンスは
[THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md) にまとめています（`python tools\gen_licenses.py` で生成）。
