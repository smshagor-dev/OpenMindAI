from pathlib import Path

path = Path("scripts/integrate_coding_workspace.py")
text = path.read_text(encoding="utf-8")
marker = "# Decision function takes plan and returns usage evidence.\n"
start = text.index(marker)
needle = "patch(\n    \"src-tauri/src/local_agent.rs\",\n    '''{{\\\"type\\\":\\\"tool\\\",\\\"tool\\\":\\\"terminal"
block_start = text.index(needle, start)
next_patch = text.index("patch(\n    \"src-tauri/src/local_agent.rs\",\n    '''- Never claim", block_start)
replacement = r'''local_path = Path("src-tauri/src/local_agent.rs")
local_text = local_path.read_text(encoding="utf-8")
terminal_marker = '{{\\"type\\":\\"tool\\",\\"tool\\":\\"terminal\\"'
position = local_text.index(terminal_marker)
line_end = local_text.index("\n", position) + 1
delivery_line = '{{\\"type\\":\\"tool\\",\\"tool\\":\\"delivery\\",\\"operation\\":\\"branches|pull_request|checks|check_jobs|check_logs|create_branch|commit_files|create_pull_request|update_pull_request|rerun_checks|merge_pull_request\\",\\"params\\":{{}}}}\\n\\\\\n'
local_text = local_text[:line_end] + delivery_line + local_text[line_end:]
local_path.write_text(local_text, encoding="utf-8")
'''
text = text[:block_start] + replacement + text[next_patch:]
path.write_text(text, encoding="utf-8")
print("coding bootstrap normalized")
