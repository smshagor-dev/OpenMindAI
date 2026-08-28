from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if old not in text:
        raise SystemExit(f"{label} anchor not found")
    return text.replace(old, new, 1)


def replace_function_by_braces(text: str, start_marker: str, replacement: str, label: str) -> str:
    start = text.find(start_marker)
    if start < 0:
        raise SystemExit(f"{label} start not found")
    brace_start = text.find("{", start)
    if brace_start < 0:
        raise SystemExit(f"{label} opening brace not found")
    depth = 0
    end = None
    for index in range(brace_start, len(text)):
        character = text[index]
        if character == "{":
            depth += 1
        elif character == "}":
            depth -= 1
            if depth == 0:
                end = index + 1
                break
    if end is None:
        raise SystemExit(f"{label} closing brace not found")
    return text[:start] + replacement + text[end:]


def patch_chat() -> None:
    chat = Path("src/lib/chat.ts")
    text = chat.read_text(encoding="utf-8")

    interface_anchor = "  contentPreview: string | null;\n}\n"
    image_support = interface_anchor + '''
const MAX_VISION_IMAGE_INPUT_BYTES = 16 * 1024 * 1024;
const MAX_VISION_IMAGE_DATA_URL_CHARS = 6_000_000;
const MAX_VISION_IMAGE_DIMENSION = 2048;
const SUPPORTED_VISION_IMAGE_TYPES = new Set(["image/jpeg", "image/png", "image/webp"]);

function visionMimeType(file: File) {
  const declared = file.type.toLowerCase();
  if (SUPPORTED_VISION_IMAGE_TYPES.has(declared)) return declared;
  if (/\\.png$/i.test(file.name)) return "image/png";
  if (/\\.jpe?g$/i.test(file.name)) return "image/jpeg";
  if (/\\.webp$/i.test(file.name)) return "image/webp";
  return null;
}

async function encodeVisionImage(file: File) {
  const mimeType = visionMimeType(file);
  if (!mimeType) {
    throw new Error("OpenMindAI Lens currently accepts PNG, JPEG, and WebP images.");
  }
  if (file.size > MAX_VISION_IMAGE_INPUT_BYTES) {
    throw new Error("Image is too large for local vision. Use an image smaller than 16 MB.");
  }

  const objectUrl = window.URL.createObjectURL(file);
  try {
    const image = document.createElement("img");
    await new Promise<void>((resolve, reject) => {
      image.onload = () => resolve();
      image.onerror = () => reject(new Error("Could not decode the selected image."));
      image.src = objectUrl;
    });

    const scale = Math.min(
      1,
      MAX_VISION_IMAGE_DIMENSION / Math.max(image.naturalWidth, image.naturalHeight, 1),
    );
    const width = Math.max(1, Math.round(image.naturalWidth * scale));
    const height = Math.max(1, Math.round(image.naturalHeight * scale));
    const canvas = document.createElement("canvas");
    canvas.width = width;
    canvas.height = height;
    const context = canvas.getContext("2d");
    if (!context) throw new Error("Could not prepare the image for local vision.");
    context.drawImage(image, 0, 0, width, height);

    let dataUrl =
      mimeType === "image/png"
        ? canvas.toDataURL("image/png")
        : canvas.toDataURL("image/jpeg", 0.88);

    if (dataUrl.length > MAX_VISION_IMAGE_DATA_URL_CHARS) {
      const flattened = document.createElement("canvas");
      flattened.width = width;
      flattened.height = height;
      const flattenedContext = flattened.getContext("2d");
      if (!flattenedContext) throw new Error("Could not compress the image for local vision.");
      flattenedContext.fillStyle = "#ffffff";
      flattenedContext.fillRect(0, 0, width, height);
      flattenedContext.drawImage(canvas, 0, 0);
      dataUrl = flattened.toDataURL("image/jpeg", 0.82);
    }

    if (dataUrl.length > MAX_VISION_IMAGE_DATA_URL_CHARS) {
      throw new Error("Image remains too large after local optimization. Resize it and try again.");
    }

    const safeName = file.name.split("]").join(" ").replace(/[\\r\\n]/g, " ");
    return `![${safeName}](${dataUrl})`;
  } finally {
    window.URL.revokeObjectURL(objectUrl);
  }
}
'''
    text = replace_once(text, interface_anchor, image_support, "AttachmentDraft")

    image_start_marker = '  } else if (kind === "image") {'
    pdf_start_marker = '  } else if (kind === "pdf") {'
    image_start = text.find(image_start_marker)
    pdf_start = text.find(pdf_start_marker, image_start)
    if image_start < 0 or pdf_start < 0:
        raise SystemExit("image attachment block anchors not found")
    image_block = '''  } else if (kind === "image") {
    contentPreview = await encodeVisionImage(file);
'''
    text = text[:image_start] + image_block + text[pdf_start:]

    kind_function = 'export function attachmentKind(file: File): AttachmentDraft["kind"] {'
    kind_start = text.find(kind_function)
    if kind_start < 0:
        raise SystemExit("attachmentKind function not found")
    old_kind = '  if (file.type.startsWith("image/")) return "image";'
    kind_line = text.find(old_kind, kind_start)
    if kind_line < 0:
        raise SystemExit("attachmentKind image line not found")
    new_kind = '  if (file.type.startsWith("image/") || /\\.(png|jpe?g|webp)$/i.test(file.name)) return "image";'
    text = text[:kind_line] + new_kind + text[kind_line + len(old_kind):]

    chat.write_text(text, encoding="utf-8")


def patch_app() -> None:
    app = Path("src/App.tsx")
    text = app.read_text(encoding="utf-8")
    replacement = '''  async function addFiles(files: FileList | null) {
    if (!files) return;
    try {
      const next = await Promise.all(Array.from(files).map(readAttachment));
      setAttachments((items) => [...items, ...next]);
      setView("chat");
      composerRef.current?.focus();
    } catch (caught) {
      showError(caught);
    }
  }'''
    text = replace_function_by_braces(
        text,
        "  async function addFiles(files: FileList | null) {",
        replacement,
        "addFiles",
    )
    app.write_text(text, encoding="utf-8")


def patch_inference() -> None:
    inference = Path("src-tauri/src/inference.rs")
    text = inference.read_text(encoding="utf-8")
    old_parse = "        let (text, images) = extract_inline_data_images(&message.content)?;"
    new_parse = '''        let (text, images) = if message.role == "user" {
            extract_inline_data_images(&message.content)?
        } else {
            (message.content.clone(), Vec::new())
        };'''
    text = replace_once(text, old_parse, new_parse, "build_context image parsing")
    inference.write_text(text, encoding="utf-8")


patch_chat()
patch_app()
patch_inference()
