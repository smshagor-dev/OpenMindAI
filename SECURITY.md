# Security Policy

OpenMindAI handles local files, model downloads, update metadata, subprocesses, and persistent user data. Security reports are taken seriously, especially when an issue could affect code execution, update integrity, local data confidentiality, path handling, or privilege boundaries.

## Supported versions

Security fixes are prioritized for the current release line and the latest code on `main`. Older releases may not receive backported fixes unless the impact justifies it.

## Reporting a vulnerability

Please do not open a public GitHub issue for a suspected vulnerability.

Use GitHub's private vulnerability reporting / Security Advisory flow for this repository when available. Include enough detail to reproduce and assess the issue safely:

- affected version or commit;
- operating system and architecture;
- installation method;
- clear reproduction steps;
- expected and observed behavior;
- security impact;
- proof-of-concept details, logs, or screenshots where useful;
- any mitigation you have already identified.

Do not include real secrets, personal data, private model files, authentication material, or data belonging to other people in a report.

## Scope

Examples of issues that should be reported privately include:

- arbitrary command or code execution;
- update or installer signature/integrity bypasses;
- unsafe path traversal or writes outside the selected OpenMindAI data root;
- disclosure or corruption of local chat/project data;
- malicious model/runtime package handling;
- privilege escalation;
- unsafe IPC boundaries or Tauri command exposure;
- dependency vulnerabilities that are practically exploitable in OpenMindAI.

General bugs, performance problems, unsupported hardware, model quality issues, or feature requests can use the normal public issue tracker unless they also create a security impact.

## Coordinated disclosure

Please allow reasonable time to reproduce, patch, validate, and release a fix before publishing vulnerability details. The project will aim to keep reporters informed as the issue is triaged and resolved.
