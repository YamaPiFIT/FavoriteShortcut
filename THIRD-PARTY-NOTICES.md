# 第三者ソフトウェアのライセンス表記

このアプリケーション（配布される `FavoriteShortcut.exe` を含む）には、
以下の第三者ソフトウェアが含まれています。いずれも再配布が許可されています。

---

## .NET 8 ランタイム / Windows Presentation Foundation (WPF)

- 提供元: Microsoft Corporation
- ライセンス: MIT License
- https://github.com/dotnet/runtime/blob/main/LICENSE.TXT
- https://github.com/dotnet/wpf/blob/main/LICENSE.TXT

自己完結版の EXE には .NET ランタイムが同梱されています。
ランタイム依存版には含まれません。

```
The MIT License (MIT)

Copyright (c) .NET Foundation and Contributors

All rights reserved.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

---

## Microsoft.Data.Sqlite (8.0.8)

- 提供元: Microsoft Corporation (.NET Foundation)
- ライセンス: MIT License
- https://github.com/dotnet/efcore/blob/main/LICENSE.txt

ライセンス全文は上記 .NET ランタイムと同一の MIT License です。

---

## SQLitePCLRaw (2.1.6)

`SQLitePCLRaw.bundle_e_sqlite3` / `.core` / `.lib.e_sqlite3` / `.provider.e_sqlite3`

- 提供元: Eric Sink / SourceGear LLC
- ライセンス: Apache License 2.0
- https://github.com/ericsink/SQLitePCL.raw/blob/master/LICENSE.TXT

```
Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```

---

## SQLite

- 提供元: SQLite Development Team (D. Richard Hipp ほか)
- ライセンス: パブリックドメイン
- https://www.sqlite.org/copyright.html

```
The author disclaims copyright to this source code. In place of a legal
notice, here is a blessing:

    May you do good and not evil.
    May you find forgiveness for yourself and forgive others.
    May you share freely, never taking more than you give.
```

---

## アプリケーションアイコン

`src/FavoriteShortcut/Assets/app.ico` および
`src/FavoriteShortcut/Services/DefaultIcons.cs` のベクターアイコンは、
本プロジェクトのために作成したオリジナルで、本体と同じ MIT License が適用されます。

なお、ショートカットに表示される favicon やファイルアイコンは、
利用者の環境で実行時に取得・キャッシュされるものであり、
本ソフトウェアに同梱・再配布されるものではありません。
