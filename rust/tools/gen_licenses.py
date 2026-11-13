"""Rust 版に組み込まれるライブラリ（crate）のライセンス一覧を THIRD-PARTY-LICENSES.md に書き出す。

使い方（rust フォルダで実行）:
    python tools/gen_licenses.py

cargo metadata で Windows 向けの依存関係を調べ、各 crate に含まれるライセンス本文を集める。
同じ本文は 1 回だけ載せ、どの crate のものかを並べる。依存関係を更新したら作り直すこと。
"""

import hashlib
import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
OUTPUT = os.path.join(ROOT, 'THIRD-PARTY-LICENSES.md')
LICENSE_FILE = re.compile(r'^(licen[cs]e|copying|copyright|notice|unlicense)', re.IGNORECASE)


def metadata():
    text = subprocess.check_output(
        ['cargo', 'metadata', '--format-version', '1', '--filter-platform', 'x86_64-pc-windows-msvc'],
        cwd=ROOT,
    )
    return json.loads(text)


def runtime_packages(meta):
    """アプリ本体から通常の依存（ビルド時だけ・テスト時だけのものを除く）でたどれる crate。"""
    packages = {p['id']: p for p in meta['packages']}
    nodes = {n['id']: n for n in meta['resolve']['nodes']}
    root = meta['resolve']['root']
    seen, stack = set(), [root]
    while stack:
        current = stack.pop()
        if current in seen:
            continue
        seen.add(current)
        for dep in nodes[current]['deps']:
            if any(kind['kind'] is None for kind in dep['dep_kinds']):
                stack.append(dep['pkg'])
    seen.discard(root)
    return sorted((packages[i] for i in seen), key=lambda p: (p['name'].lower(), p['version']))


def license_files(package):
    directory = os.path.dirname(package['manifest_path'])
    found = []
    for name in sorted(os.listdir(directory)):
        path = os.path.join(directory, name)
        if os.path.isfile(path) and LICENSE_FILE.match(name):
            found.append(path)
    # egui の既定フォント（アプリに組み込まれる）のライセンス
    fonts = os.path.join(directory, 'fonts')
    if package['name'] == 'epaint_default_fonts' and os.path.isdir(fonts):
        for name in sorted(os.listdir(fonts)):
            if name.lower().endswith('.txt'):
                found.append(os.path.join(fonts, name))
    return found


def read_text(path):
    with open(path, 'rb') as f:
        data = f.read()
    text = data.decode('utf-8', errors='replace').replace('\r\n', '\n').replace('\r', '\n')
    return '\n'.join(line.rstrip() for line in text.strip('\n').split('\n'))


def main():
    packages = runtime_packages(metadata())
    texts = {}  # hash -> (text, [crate names])
    order = []
    missing = []
    for p in packages:
        files = license_files(p)
        if not files:
            missing.append(p)
        for path in files:
            text = read_text(path)
            key = hashlib.sha1(text.encode('utf-8')).hexdigest()
            if key not in texts:
                texts[key] = (text, [])
                order.append(key)
            label = f"{p['name']} {p['version']}"
            if os.path.basename(os.path.dirname(path)) == 'fonts':
                label += f" ({os.path.basename(path)})"
            texts[key][1].append(label)

    lines = [
        '# Rust 版のライブラリのライセンス',
        '',
        'Rust 版（`FavoriteShortcutRs.exe`）には、以下のライブラリ（crate）が組み込まれています。',
        'いずれも再配布が許可されたライセンスです。複数のライセンスから選べるもの（`MIT OR Apache-2.0` など）は、',
        'そのいずれかに従って利用しています。',
        '',
        'このファイルは `tools/gen_licenses.py` で自動生成しています（依存関係を更新したら作り直してください）。',
        '',
        '補足:',
        '',
        '- SQLite は `libsqlite3-sys` に同梱されたものを組み込んでいます（パブリックドメイン）。',
        '- `epaint_default_fonts` のフォント（Ubuntu Light・Hack・Noto Emoji ほか）が EXE に組み込まれます。'
        'それぞれのライセンス本文は下記に含まれています。',
        '- 画面の日本語表示には Windows に入っているフォント（Yu Gothic UI など）を実行時に読み込んで使います。'
        'フォントファイルは同梱・再配布しません。',
        '',
        f'## 一覧（{len(packages)} 件）',
        '',
        '| crate | バージョン | ライセンス | 入手先 |',
        '| --- | --- | --- | --- |',
    ]
    for p in packages:
        repo = p.get('repository') or p.get('homepage') or ''
        license_name = (p.get('license') or '（パッケージ内のファイルを参照）').replace('|', '/')
        lines.append(f"| {p['name']} | {p['version']} | {license_name} | {repo} |")

    lines += ['', '## ライセンス本文', '']
    for key in order:
        text, owners = texts[key]
        lines.append('### ' + ', '.join(owners[:6]) + (f' ほか {len(owners) - 6} 件' if len(owners) > 6 else ''))
        lines.append('')
        if len(owners) > 6:
            lines.append('対象: ' + ', '.join(owners))
            lines.append('')
        lines.append('```text')
        lines.append(text.replace('```', "'''"))
        lines.append('```')
        lines.append('')

    if missing:
        lines += ['## ライセンス本文がパッケージに含まれていない crate', '',
                  '以下は配布パッケージにライセンスのファイルが含まれていないため、表記されたライセンス（SPDX）のみを記載します。', '']
        for p in missing:
            lines.append(f"- {p['name']} {p['version']}: {p.get('license') or '不明'}")
        lines.append('')

    with open(OUTPUT, 'w', encoding='utf-8', newline='\n') as f:
        f.write('\n'.join(lines))
    print(f'{OUTPUT}: crates {len(packages)}, unique texts {len(order)}, missing {len(missing)}')


if __name__ == '__main__':
    sys.exit(main())
