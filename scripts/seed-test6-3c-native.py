"""Seed a disposable native export fixture after the app has initialized its DB."""
import argparse
import hashlib
import json
import sqlite3
import uuid
from datetime import datetime, timezone
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("--root", required=True, type=Path)
parser.add_argument("--article", type=Path, default=Path(__file__).parents[1] / "artifacts/acceptance/test6-2026-11-23/evidence-analysis/article-draft.md")
parser.add_argument("--output", required=True, type=Path)
args = parser.parse_args()
root = args.root.resolve()
db_path = root / "data/database/openmind_ai.db"
if not db_path.is_file() or not args.article.is_file():
    raise SystemExit("Expected the initialized disposable database and evidence-limited article draft.")
content = args.article.read_text(encoding="utf-8")
now = datetime.now(timezone.utc).isoformat()
project_id, run_id, conversation_id, revision_id = [str(uuid.uuid4()) for _ in range(4)]
digest = hashlib.sha256(content.encode()).hexdigest()
title = "Test 6 Milestone 3C Figure Export (Disposable)"
prompt = "Disposable deterministic export fixture from the reviewed, evidence-limited MNIST draft. No model inference was run."
with sqlite3.connect(db_path) as db:
    db.execute("PRAGMA foreign_keys=ON")
    db.execute("INSERT OR IGNORE INTO model_registry(id,name,path,format,size_bytes,capabilities,enabled,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)", ("test6-3c-no-model", "Milestone 3C fixture (no model)", "models/__test6_3c_fixture_missing__.gguf", "gguf", 0, "[]", 0, now, now))
    db.execute("INSERT INTO research_projects(id,title,prompt,output_language,status,created_at,updated_at,experiment_requested,manuscript_target_words,manuscript_page_target) VALUES(?,?,?,?,?,?,?,?,?,?)", (project_id, title, prompt, "English", "draft", now, now, 0, 3500, 3))
    db.execute("INSERT INTO conversations(id,title,created_at,updated_at) VALUES(?,?,?,?)", (conversation_id, title, now, now))
    db.execute("INSERT INTO research_runs(id,project_id,conversation_id,model_id,status,started_at,updated_at,completed_at) VALUES(?,?,?,?,?,?,?,?)", (run_id, project_id, conversation_id, "test6-3c-no-model", "completed", now, now, now))
    db.execute("INSERT INTO research_manuscript_revisions(id,run_id,revision,content,content_hash,status,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?)", (revision_id, run_id, 1, content, digest, "proposal", now, now))
record = {"root": str(root), "database": str(db_path), "projectId": project_id, "runId": run_id, "conversationId": conversation_id, "revisionId": revision_id, "title": title, "draftSha256": digest, "sourceArticle": str(args.article.resolve()), "modelInstalled": False, "modelInference": False}
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
print(json.dumps(record, indent=2))
