# リリース手順

GitHub の Releases に EXE を公開して、誰でもダウンロードできるようにするための手順です。

**タグを push すると、テスト・EXE と ZIP の作成・リリースの下書き作成までを GitHub が自動で行います**
（[`.github/workflows/release.yml`](../.github/workflows/release.yml)）。
最後に内容を確認して「Publish release」を押すだけで公開されます。

## 1. バージョン番号を上げる

次の 2 か所を同じ番号にします。

- `src/FavoriteShortcut/FavoriteShortcut.csproj` の `<Version>`
- `installer/FavoriteShortcut.iss` の `AppVersion`

```xml
<Version>1.3.0</Version>
```

## 2. リリースノートを書く

`docs/release-notes-v1.3.0.md` のように、タグと同じ名前のファイルを作ります。
この内容がそのままリリースの説明文になります（過去のファイルが参考になります）。

添付する EXE / ZIP のハッシュ値（SHA256）は自動で付け足されます。
置きたい場所に `<!-- SHA256 -->` と書いておくとそこへ、書かなければ末尾に入ります。

## 3. 手元で確認する（任意）

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build.ps1 -Mode Both
```

検証プログラムが自動で実行され、失敗した場合はビルドが中止されます。成功すると次が生成されます。

```
build\self-contained\FavoriteShortcut.exe        ← 配布用（約 68 MB）
build\FavoriteShortcut-portable.zip              ← 配布用（約 62 MB）
build\framework-dependent\FavoriteShortcut.exe   ← .NET が別途必要な版（配布はしない）
```

## 4. コミットしてタグを push する

```powershell
git add -A
git commit -m "v1.3.0"
git tag -a v1.3.0 -m "お気に入りショートカット v1.3.0"
git push origin main
git push origin v1.3.0
```

タグは main とは別に push してください（同時に push したときに、自動化が起動しなかったことがあります）。
タグが届くと GitHub の **Actions** タブで「Release」が動き始めます（10 分ほど）。
テストが失敗した場合はリリースは作られません。Actions の画面でエラーを確認してください。

## 5. 下書きを確認して公開する

https://github.com/YamaPiFIT/FavoriteShortcut/releases

1. 「Draft」と表示されたリリースを開き、鉛筆アイコン（Edit）を押す
2. 説明文と、添付の `FavoriteShortcut.exe` / `FavoriteShortcut-portable.zip` を確認する
3. **Publish release** を押す

公開すると README のダウンロードバッジが自動で最新版を指すようになります。

同じタグで作り直したいときは、Actions の画面から「Re-run jobs」を実行すると、
下書きの添付ファイルと説明文が差し替わります。

---

## 自動化が使えないときの手順

手順 3 のビルドで作った EXE と ZIP を、手作業で添付します。

1. https://github.com/YamaPiFIT/FavoriteShortcut/releases/new を開く
2. **Choose a tag** で、push したタグ（例 `v1.3.0`）を選ぶ
3. **Release title** にタグと同じ名前を入れ、説明欄にリリースノートの内容を貼る
4. **Attach binaries** の枠へ EXE と ZIP をドラッグ＆ドロップ
   - アップロードのプログレスバーが 100% になるまで待つ（サイズが大きいため数分かかることがあります）
5. **Publish release** を押す

ハッシュ値は次で取得できます。

```powershell
Get-FileHash build\self-contained\FavoriteShortcut.exe -Algorithm SHA256
Get-FileHash build\FavoriteShortcut-portable.zip -Algorithm SHA256
```

---

## インストーラーも配布する場合

[Inno Setup 6](https://jrsoftware.org/isinfo.php) を入れたうえで、手順 3 のあとに実行します。

```powershell
& "C:\Program Files (x86)\Inno Setup 6\ISCC.exe" installer\FavoriteShortcut.iss
```

`installer\FavoriteShortcut-Setup.exe` ができるので、これもリリースに添付します。
