# Sidebar localization (partial #377)

The persisted Settings language now reaches Sidebar directly and translates its
navigation, profile menu, action titles and collapse/expand accessible tooltips
for English, Bangla and Hindi. Unsupported locale strings fall back to English.
The catalog is keyed, typed and centralised to allow incremental coverage.
The setting is applied on next React render after preferences save/reload.

Source-level regression tests check wiring and catalog parity. This is not a
full application translation: chat history actions, settings forms, setup,
model download, errors and other views still need localization, visual RTL/LTR
and font coverage testing. Keep #377 open. No locale-specific data leaves device.
