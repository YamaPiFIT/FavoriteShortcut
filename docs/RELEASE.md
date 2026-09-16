# リリース手順

GitHub の Releases に EXE を公開して、誰でもダウンロードできるようにするための手順です。

## 1. バージョン番号を上げる

`src/FavoriteShortcut/FavoriteShortcut.csproj` の `<Version>` を更新します。

```xml
<Version>1.0.1</Version>
```

## 2. ビルドする

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build.ps1 -Mode Both
```

検証プログラム（70 件）が自動で実行され、失敗した場合はビルドが中止されます。
成功すると次が生成されます。

```
build\self-contained\FavoriteShortcut.exe        ← Releases に添付する（約 68 MB）
build\FavoriteShortcut-portable.zip              ← Releases に添付する（約 62 MB）
build\framework-dependent\FavoriteShortcut.exe   ← 添付しない（.NET が別途必要なため）
```

## 3. コミットしてタグを打つ

```powershell
git add -A
git commit -m "v1.0.1"
git tag -a v1.0.1 -m "お気に入りショートカット v1.0.1"
git push origin main --follow-tags
```

## 4. Releases を作成する

https://github.com/YamaPiFIT/FavoriteShortcut/releases/new

1. **Choose a tag** で、手順 3 で push したタグ（例 `v1.0.1`）を選ぶ
2. **Release title** に `v1.0.1` などを入れる
3. 説明欄に変更点を書く（下に雛形あり）
4. **Attach binaries** の枠へ、手順 2 の EXE と ZIP をドラッグ＆ドロップ
   - アップロードのプログレスバーが 100% になるまで待つ（サイズが大きいため数分かかることがあります）
5. **Publish release** を押す

公開すると README のダウンロードバッジが自動で最新版を指すようになります。

---

## 説明欄の雛形

```markdown
Windows 用のお気に入り管理 + ランチャーアプリです。

## ダウンロード

| ファイル | 内容 |
|---|---|
| `FavoriteShortcut.exe` | これ 1 つで動きます（.NET のインストール不要） |
| `FavoriteShortcut-portable.zip` | 上記 + README / LICENSE をまとめたもの |

## 使い方

1. ダウンロードした EXE を好きな場所に置いてダブルクリック
2. `Ctrl + Space` でランチャーが開きます
3. キーワードを入力して `Enter` で起動

登録したデータは EXE と同じ場所に作られる `Data` フォルダに保存されます。
フォルダごとコピーすれば別の PC へそのまま移せます。

## 動作環境

Windows 10 (1809 以降) / Windows 11 の 64bit

## ご注意

初回起動時に「WindowsによってPCが保護されました」と表示されます。
個人で作成した未署名のアプリのため出るもので、**詳細情報** → **実行** で起動できます。

## 変更点

- （ここに変更内容を書く）
```

---

## SHA256 を添える場合

配布物の改ざん検知用にハッシュを説明欄へ載せるときは、次で取得できます。

```powershell
Get-FileHash build\self-contained\FavoriteShortcut.exe -Algorithm SHA256
Get-FileHash build\FavoriteShortcut-portable.zip -Algorithm SHA256
```

---

## コマンドラインから公開したい場合

[GitHub CLI](https://cli.github.com/) を入れておくと、手順 4 を 1 コマンドで実行できます。

```powershell
winget install --id GitHub.cli
gh auth login          # 初回のみ。ブラウザで認証します
```

以降はビルド後に次を実行するだけです。

```powershell
gh release create v1.0.1 `
  "build\self-contained\FavoriteShortcut.exe" `
  "build\FavoriteShortcut-portable.zip" `
  --title "v1.0.1" `
  --notes-file docs\release-notes.md
```

---

## インストーラーも配布する場合

[Inno Setup 6](https://jrsoftware.org/isinfo.php) を入れたうえで、手順 2 のあとに実行します。

```powershell
& "C:\Program Files (x86)\Inno Setup 6\ISCC.exe" installer\FavoriteShortcut.iss
```

`installer\FavoriteShortcut-Setup.exe` ができるので、これも Releases に添付します。
