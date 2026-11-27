"""Bounded structural and media verification for a native figure-bearing DOCX."""
import hashlib
import json
import sys
import zipfile
import xml.etree.ElementTree as ET
from pathlib import Path
from PIL import Image
from io import BytesIO

docx = Path(sys.argv[1])
report_path = Path(sys.argv[2])
with zipfile.ZipFile(docx) as archive:
    names = set(archive.namelist())
    required = {"[Content_Types].xml", "word/document.xml", "word/_rels/document.xml.rels"}
    if not required.issubset(names):
        raise SystemExit(f"Missing OOXML parts: {sorted(required - names)}")
    document = ET.fromstring(archive.read("word/document.xml"))
    relationships = ET.fromstring(archive.read("word/_rels/document.xml.rels"))
    content_types = ET.fromstring(archive.read("[Content_Types].xml"))
    rel_ns = "{http://schemas.openxmlformats.org/package/2006/relationships}"
    ns = {"w": "http://schemas.openxmlformats.org/wordprocessingml/2006/main", "a": "http://schemas.openxmlformats.org/drawingml/2006/main", "r": "http://schemas.openxmlformats.org/officeDocument/2006/relationships", "wp": "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"}
    rels = {rel.attrib["Id"]: rel.attrib["Target"] for rel in relationships.findall(rel_ns + "Relationship")}
    defaults = {node.attrib.get("Extension"): node.attrib.get("ContentType") for node in content_types}
    drawings = document.findall(".//w:drawing", ns)
    embedded = []
    for drawing in drawings:
        blip = drawing.find(".//a:blip", ns)
        if blip is None:
            raise SystemExit("A drawing node has no image relationship reference.")
        rid = blip.attrib["{" + ns["r"] + "}embed"]
        target = rels.get(rid)
        if not target or not target.startswith("media/"):
            raise SystemExit(f"Image relationship {rid} is missing or unsafe: {target}")
        member = "word/" + target
        if member not in names:
            raise SystemExit(f"Missing image media part {member}")
        raw = archive.read(member)
        with Image.open(BytesIO(raw)) as image:
            image.verify()
        with Image.open(BytesIO(raw)) as image:
            embedded.append({"relationshipId": rid, "member": member, "mediaSha256": hashlib.sha256(raw).hexdigest(), "format": image.format, "width": image.width, "height": image.height, "alt": drawing.find(".//wp:docPr", ns).attrib.get("descr")})
    paragraphs = []
    body = document.find(".//w:body", ns)
    for child in list(body):
        if child.tag.endswith("}p"):
            paragraphs.append("".join(node.text or "" for node in child.findall(".//w:t", ns)))
        elif child.tag.endswith("}tbl"):
            paragraphs.append("TABLE")
    captions = [text for text in paragraphs if "Figure 1. Held-out client accuracy by round" in text]
    referenced = {item["relationshipId"] for item in embedded}
    if len(embedded) < 1 or not captions or not any(target in names for target in ("word/" + value for rid, value in rels.items() if rid in referenced)):
        raise SystemExit("DOCX is missing figure media, a valid relationship, or its caption.")
    report = {"docxPath": str(docx), "docxSha256": hashlib.sha256(docx.read_bytes()).hexdigest(), "requiredPartsPresent": True, "drawingCount": len(drawings), "relationshipCount": len(rels), "pngContentType": defaults.get("png"), "embeddedImages": embedded, "captionParagraphs": captions, "captionParagraphPositions": [i for i, paragraph in enumerate(paragraphs) if "Figure 1." in paragraph or "Figure 2." in paragraph], "imageParagraphPositions": [i for i, node in enumerate(list(body)) if node.find(".//w:drawing", ns) is not None], "packageEntries": sorted(names)}
report_path.parent.mkdir(parents=True, exist_ok=True)
report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
print(json.dumps(report, indent=2))
