from pathlib import Path

path = Path("scripts/bootstrap_runtime_guards.py")
text = path.read_text(encoding="utf-8")
label = '    "mac scratch profile",\n)'
pos = text.find(label)
if pos < 0:
    raise SystemExit("mac scratch profile patch label not found")
start = text.rfind("runtime = replace_once(", 0, pos)
end = text.find("runtime = replace_once(", pos + len(label))
if start < 0 or end < 0:
    raise SystemExit("mac scratch profile patch span not found")
replacement = r'''mac_fn = runtime.find("async fn run_macos_sandbox(")
profile_start = runtime.find("    let profile = format!(\n", mac_fn)
profile_end = runtime.find("    let wrapped = shell_wrapper(command);\n", profile_start)
if mac_fn < 0 or profile_start < 0 or profile_end < 0:
    raise SystemExit("mac sandbox profile span not found")
new_profile = '''    let profile = format!(
        "(version 1)\\n\\
         (deny default)\\n\\
         (allow process*)\\n\\
         (allow signal)\\n\\
         (allow sysctl-read)\\n\\
         (allow mach-lookup)\\n\\
         (allow ipc-posix*)\\n\\
         (allow file-read-metadata)\\n\\
         (allow file-read*\\n\\
           (subpath \\\"/System\\\")\\n\\
           (subpath \\\"/usr\\\")\\n\\
           (subpath \\\"/bin\\\")\\n\\
           (subpath \\\"/sbin\\\")\\n\\
           (subpath \\\"/Library\\\")\\n\\
           (subpath \\\"/Applications/Xcode.app\\\")\\n\\
           (subpath \\\"/private/etc\\\")\\n\\
           (subpath \\\"/private/var/db/dyld\\\")\\n\\
           (subpath \\\"/dev\\\")\\n\\
           (subpath \\\"{workspace}\\\")\\n\\
           (subpath \\\"{scratch}\\\"))\\n\\
         (allow file-write*\\n\\
           (subpath \\\"{workspace}\\\")\\n\\
           (subpath \\\"{scratch}\\\"))\\n\\
         (deny network*)"
    );
'''
runtime = runtime[:profile_start] + new_profile + runtime[profile_end:]
'''
text = text[:start] + replacement + text[end:]
path.write_text(text, encoding="utf-8")
