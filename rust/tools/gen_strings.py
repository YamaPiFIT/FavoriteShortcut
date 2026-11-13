"""C# 版の対訳表（src/FavoriteShortcut/Services/Strings.cs）から Rust の対訳表を作る。

  python rust/tools/gen_strings.py

C# 版と Rust 版で画面の文言を完全に揃えるため、手で写さずにこのスクリプトで変換する。
出力: rust/src/strings_table.rs
"""
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.normpath(os.path.join(HERE, "..", "..", "src", "FavoriteShortcut", "Services", "Strings.cs"))
OUT = os.path.normpath(os.path.join(HERE, "..", "src", "strings_table.rs"))


def parse_csharp_literals(text):
    """表の部分を読み、(key, ja, en) の並びを返す。"""
    start = text.index("Table =")
    start = text.index("{", start) + 1
    end = text.index("};", start)
    body = text[start:end]

    i = 0
    n = len(body)
    entries = []

    def skip_ws_and_comments(i):
        while i < n:
            if body[i].isspace():
                i += 1
            elif body.startswith("//", i):
                i = body.index("\n", i) + 1 if "\n" in body[i:] else n
            else:
                break
        return i

    def read_literal(i):
        verbatim = False
        if body[i] == "@":
            verbatim = True
            i += 1
        if body[i] != '"':
            raise ValueError(f"文字列が来るはずの位置: {body[i:i+40]!r}")
        i += 1
        out = []
        while True:
            c = body[i]
            if verbatim:
                if c == '"':
                    if i + 1 < n and body[i + 1] == '"':
                        out.append('"')
                        i += 2
                        continue
                    return "".join(out), i + 1
                out.append(c)
                i += 1
                continue
            if c == '"':
                return "".join(out), i + 1
            if c == "\\":
                e = body[i + 1]
                if e == "n":
                    out.append("\n")
                elif e == "r":
                    out.append("\r")
                elif e == "t":
                    out.append("\t")
                elif e == '"':
                    out.append('"')
                elif e == "\\":
                    out.append("\\")
                elif e == "'":
                    out.append("'")
                elif e == "0":
                    out.append("\0")
                elif e == "u":
                    out.append(chr(int(body[i + 2:i + 6], 16)))
                    i += 4
                else:
                    raise ValueError(f"未対応のエスケープ: \\{e}")
                i += 2
                continue
            out.append(c)
            i += 1

    def read_expr(i):
        """"..." + "..." のような連結も 1 つの文字列として読む。"""
        parts = []
        while True:
            i = skip_ws_and_comments(i)
            value, i = read_literal(i)
            parts.append(value)
            i = skip_ws_and_comments(i)
            if body[i] == "+":
                i += 1
                continue
            return "".join(parts), i

    while True:
        i = skip_ws_and_comments(i)
        if i >= n:
            break
        if body[i] != "(":
            raise ValueError(f"( が来るはずの位置: {body[i:i+40]!r}")
        i += 1
        values = []
        for k in range(3):
            value, i = read_expr(i)
            values.append(value)
            i = skip_ws_and_comments(i)
            expected = "," if k < 2 else ")"
            if body[i] != expected:
                raise ValueError(f"{expected} が来るはずの位置: {body[i:i+40]!r}")
            i += 1
        entries.append(tuple(values))
        i = skip_ws_and_comments(i)
        if i < n and body[i] == ",":
            i += 1
    return entries


def rust_literal(s):
    out = ['"']
    for c in s:
        if c == "\\":
            out.append("\\\\")
        elif c == '"':
            out.append('\\"')
        elif c == "\n":
            out.append("\\n")
        elif c == "\r":
            out.append("\\r")
        elif c == "\t":
            out.append("\\t")
        elif c == "\0":
            out.append("\\0")
        else:
            out.append(c)
    out.append('"')
    return "".join(out)


def main():
    text = open(SRC, encoding="utf-8").read()
    entries = parse_csharp_literals(text)
    keys = [e[0] for e in entries]
    dup = {k for k in keys if keys.count(k) > 1}
    if dup:
        sys.exit(f"キーが重複しています: {sorted(dup)}")

    lines = [
        "// このファイルは tools/gen_strings.py が C# 版の Strings.cs から生成したものです。手で編集しないでください。",
        "// 文言を変えるときは C# 版の対訳表を直してから、スクリプトを実行し直します。",
        "",
        "/// （キー, 日本語, 英語）",
        "pub static TABLE: &[(&str, &str, &str)] = &[",
    ]
    for key, ja, en in entries:
        lines.append(f"    ({rust_literal(key)}, {rust_literal(ja)}, {rust_literal(en)}),")
    lines.append("];")
    lines.append("")

    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(lines))
    print(f"{len(entries)} 件を書き出しました: {OUT}")


if __name__ == "__main__":
    main()
