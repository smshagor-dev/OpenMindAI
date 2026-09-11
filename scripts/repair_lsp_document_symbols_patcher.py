from pathlib import Path

path = Path("scripts/apply_lsp_document_symbols.py")
text = path.read_text()
anchor = "local = local_path.read_text()\n"
base = text.index(anchor) + len(anchor)
start = text.index("local = replace_once(\n", base)
marker = '    "symbol outline prompt shape",\n)\n'
end = text.index(marker, start) + len(marker)
replacement = '''local_lines = local.splitlines(keepends=True)\nmatches = [\n    index\n    for index, line in enumerate(local_lines)\n    if "symbol_search" in line and "symbol name" in line\n]\nif len(matches) != 1:\n    raise SystemExit(f"expected one symbol_search prompt line, found {len(matches)}")\nindex = matches[0]\noutline_line = (\n    local_lines[index]\n    .replace("symbol_search", "symbol_outline")\n    .replace("query", "path")\n    .replace("symbol name", "file")\n)\nlocal_lines.insert(index + 1, outline_line)\nlocal = ''.join(local_lines)\n'''
path.write_text(text[:start] + replacement + text[end:])
