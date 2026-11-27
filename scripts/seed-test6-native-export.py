"""Seed a clearly labeled export fixture in the disposable Test 6 database.

This stores the already-reviewed evidence-limited Markdown draft as a draft
revision so the running Tauri app's real PDF/DOCX export command can be tested.
It does not create an AI run or claim the manuscript was model-generated.
"""
import hashlib
import json
import sqlite3
import uuid
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(r"D:\OpenMindAI\artifacts\acceptance\test6-2026-11-23\app-root-creation-ui")
DB = ROOT / "data/database/openmind_ai.db"
ARTICLE = Path(r"D:\OpenMindAI\artifacts\acceptance\test6-2026-11-23\evidence-analysis\article-draft.md")
OUT = Path(r"D:\OpenMindAI\artifacts\acceptance\test6-2026-11-23\native-export-fixture.json")

if not DB.is_file() or not ARTICLE.is_file():
    raise SystemExit("Expected the disposable Test 6 database and existing evidence-limited draft.")
content = ARTICLE.read_text(encoding="utf-8")
now = datetime.now(timezone.utc).isoformat()
project_id, run_id, conversation_id, revision_id = [str(uuid.uuid4()) for _ in range(4)]
digest = hashlib.sha256(content.encode("utf-8")).hexdigest()
title = "Test 6 Export Acceptance — Evidence-Limited Draft"
prompt = "Disposable deterministic export fixture for the supplied single-seed MNIST evidence-limited draft. No model inference was run."

with sqlite3.connect(DB) as db:
    db.execute("PRAGMA foreign_keys=ON")
    # This purpose-built completed export fixture gets a disabled registry row
    # with no backing file so the run stays referentially valid and cannot be
    # selected or mistaken for an installed model.
    db.execute("INSERT OR IGNORE INTO model_registry(id,name,path,format,size_bytes,capabilities,enabled,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)", ("export-fixture-no-model", "Test 6 export fixture (model absent)", "models/__test6_export_fixture_missing__.gguf", "gguf", 0, "[]", 0, now, now))
    db.execute("INSERT INTO research_projects(id,title,prompt,output_language,status,created_at,updated_at,experiment_requested,manuscript_target_words,manuscript_page_target) VALUES(?,?,?,?,?,?,?,?,?,?)", (project_id,title,prompt,"English","draft",now,now,0,3500,24))
    db.execute("INSERT INTO conversations(id,title,created_at,updated_at) VALUES(?,?,?,?)", (conversation_id,title,now,now))
    db.execute("INSERT INTO research_runs(id,project_id,conversation_id,model_id,status,started_at,updated_at,completed_at) VALUES(?,?,?,?,?,?,?,?)", (run_id,project_id,conversation_id,"export-fixture-no-model","completed",now,now,now))
    db.execute("INSERT INTO research_manuscript_revisions(id,run_id,revision,content,content_hash,status,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?)", (revision_id,run_id,1,content,digest,"proposal",now,now))

OUT.write_text(json.dumps({"database": str(DB), "projectId": project_id, "runId": run_id, "conversationId": conversation_id, "revisionId": revision_id, "title": title, "draftSha256": digest, "sourceArticle": str(ARTICLE), "modelId": "export-fixture-no-model", "modelInstalled": False, "modelInference": False}, indent=2) + "\n", encoding="utf-8")
print(OUT.read_text(encoding="utf-8"))
