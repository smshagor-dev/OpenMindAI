import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ModelRecord } from "../types";

type FigureAsset = {
  id: string;
  projectId: string;
  name: string;
  mediaType: string;
  sizeBytes: number;
  contentSha256: string;
  widthPx: number;
  heightPx: number;
  caption: string;
  altText: string;
  importedAt: string;
};

// Canonical figure reference used by the export resolver.
const FIGURE_REF = /!\[([^\]]*)\]\(research-figure:([0-9a-fA-F-]{36})(?:\s+"((?:[^"\\]|\\.)*)")?\)/g;
type FigureRef = { alt: string; id: string; caption: string; raw: string; start: number; end: number };

function parseFigureRefs(text: string): FigureRef[] {
  const refs: FigureRef[] = [];
  FIGURE_REF.lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = FIGURE_REF.exec(text)) !== null) {
    refs.push({ alt: match[1] ?? "", id: match[2], caption: (match[3] ?? "").replace(/\\"/g, '"'), raw: match[0], start: match.index, end: match.index + match[0].length });
  }
  return refs;
}

function stripFigureLabel(caption: string): string {
  return caption.replace(/^\s*figure\s+\d+\s*[.:]?\s*/i, "").trim();
}

function buildFigureRef(alt: string, id: string, caption: string): string {
  const safeCaption = caption.replace(/"/g, '\\"');
  return `![${alt}](research-figure:${id} "${safeCaption}")`;
}

// Renumber every figure caption to "Figure N. <base>" in document order so the
// placement order and Figure N labels always agree.
function renumberFigures(text: string): string {
  let n = 0;
  return text.replace(FIGURE_REF, (_whole, alt: string, id: string, caption: string | undefined) => {
    n += 1;
    const base = stripFigureLabel((caption ?? "").replace(/\\"/g, '"'));
    const numbered = base ? `Figure ${n}. ${base}` : `Figure ${n}.`;
    return buildFigureRef(alt, id, numbered);
  });
}

// Insert a figure reference at `cursor` (its own block) and renumber.
function insertFigureAt(text: string, cursor: number, asset: { id: string; altText: string; caption: string }): string {
  const ref = buildFigureRef(asset.altText || "Figure", asset.id, stripFigureLabel(asset.caption) || asset.caption);
  const at = Math.max(0, Math.min(cursor, text.length));
  const before = text.slice(0, at).replace(/\s*$/, "");
  const after = text.slice(at).replace(/^\s*/, "");
  const block = `${before}${before ? "\n\n" : ""}${ref}${after ? "\n\n" : ""}${after}`;
  return renumberFigures(block);
}

function removeFigureOccurrence(text: string, occurrence: number): string {
  const refs = parseFigureRefs(text);
  const target = refs[occurrence];
  if (!target) return text;
  const next = `${text.slice(0, target.start).replace(/\s*$/, "")}\n\n${text.slice(target.end).replace(/^\s*/, "")}`.trim();
  return renumberFigures(next);
}

function moveFigureOccurrence(text: string, occurrence: number, direction: -1 | 1): string {
  const refs = parseFigureRefs(text);
  const a = refs[occurrence];
  const b = refs[occurrence + direction];
  if (!a || !b) return text;
  const [left, right] = direction === 1 ? [a, b] : [b, a];
  const swapped = text.slice(0, left.start) + right.raw + text.slice(left.end, right.start) + left.raw + text.slice(right.end);
  return renumberFigures(swapped);
}

function replaceFigureOccurrence(text: string, occurrence: number, asset: { id: string; altText: string; caption: string }): string {
  const refs = parseFigureRefs(text);
  const target = refs[occurrence];
  if (!target) return text;
  const ref = buildFigureRef(asset.altText || "Figure", asset.id, stripFigureLabel(asset.caption) || asset.caption);
  return renumberFigures(text.slice(0, target.start) + ref + text.slice(target.end));
}

// Mirror of the Rust source classes in research_sections.rs.
const SOURCE_CLASSES: Array<{ value: string; label: string; numeric: boolean }> = [
  { value: "raw_experiment", label: "Raw experiment output", numeric: true },
  { value: "derived_analysis", label: "Derived analysis output", numeric: true },
  { value: "audit_interpretation", label: "Audit / interpretation", numeric: false },
  { value: "external_reference", label: "External reference", numeric: false },
  { value: "software_fixture", label: "Software fixture", numeric: false },
  { value: "unverifiable_claim", label: "Unverifiable claim", numeric: false },
];

type OutlineEntry = {
  key: string;
  heading: string;
  plannedWords: number;
  plannedPages: number;
  evidenceRequirement: string;
  status: string;
  latestRevision: number | null;
  approvedRevision: number | null;
};
type OutlinePlan = {
  projectId: string;
  pageTarget: number;
  wordsPerPageAssumption: number;
  plannedTotalWords: number;
  entries: OutlineEntry[];
  notice: string;
};
type SectionDraft = {
  id: string;
  projectId: string;
  sectionKey: string;
  heading: string;
  revision: number;
  status: string;
  origin: string;
  content: string | null;
  contentSha256: string | null;
  validation: Record<string, unknown> | null;
  error: string | null;
  modelId: string | null;
  runtimeVersion: string | null;
  promptSha256: string | null;
  outputSha256: string | null;
  evidenceFactSetSha256: string | null;
  attempts: number;
  latencyMs: number | null;
  startedAt: string;
  completedAt: string | null;
  approvedAt: string | null;
};
type LedgerRow = {
  sectionKey: string;
  revision: number;
  draftStatus: string;
  paragraphIndex: number;
  renderedToken: string | null;
  factId: string;
  documentName: string;
  documentSha256: string;
  sourceClass: string;
  locator: string;
  metric: string;
  valueText: string;
  method: string;
};
type EvidenceDocSummary = {
  id: string;
  name: string;
  mediaType: string;
  sha256: string;
  sourceClass: string | null;
  factCount: number;
};
type EvidenceInventory = {
  documents: EvidenceDocSummary[];
  facts: Array<{ id: string }>;
  factSetSha256: string;
};
type ExportResult = {
  format: string;
  contentSha256: string;
  fileSha256: string | null;
  ledgerSha256: string;
  draft: boolean;
  pageTarget: number;
  actualPages: number | null;
  wordCount: number;
  pageTargetStatus: string;
  missingSections: Array<Record<string, unknown>>;
};

function explain(error: unknown): string {
  if (typeof error === "string") return error;
  if (error && typeof error === "object" && "message" in error) return String((error as { message: unknown }).message);
  return String(error);
}

/**
 * Evidence-grounded, section-wise manuscript workflow. Every numeric claim is
 * generated by the local model only by reference to facts extracted in Rust,
 * and the claim-to-source ledger is inspectable here and in exports. No content
 * is padded to reach a page target.
 */
export function ResearchManuscript({
  projectId,
  models,
  selectedModelId,
  onOpenModelManager,
}: {
  projectId: string;
  models: ModelRecord[];
  selectedModelId: string;
  onOpenModelManager: () => void;
}) {
  const [outline, setOutline] = useState<OutlinePlan | null>(null);
  const [sections, setSections] = useState<SectionDraft[]>([]);
  const [ledger, setLedger] = useState<LedgerRow[]>([]);
  const [inventory, setInventory] = useState<EvidenceInventory | null>(null);
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [runningKey, setRunningKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [editingDraftId, setEditingDraftId] = useState<string | null>(null);
  const [editDraftText, setEditDraftText] = useState("");
  const [exportResult, setExportResult] = useState<ExportResult | null>(null);
  const [showLedger, setShowLedger] = useState(false);
  const [figures, setFigures] = useState<FigureAsset[]>([]);
  const [figurePreviews, setFigurePreviews] = useState<Record<string, string>>({});
  const [pickerOpen, setPickerOpen] = useState(false);
  const [pickerReplaceIndex, setPickerReplaceIndex] = useState<number | null>(null);
  const [figureQuery, setFigureQuery] = useState("");
  const editTextareaRef = useRef<HTMLTextAreaElement | null>(null);

  const modelAvailable = Boolean(selectedModelId) && models.some((model) => model.id === selectedModelId && model.enabled && ["installed", "ready", "loaded"].includes(model.state));
  const numericFactCount = inventory?.facts.length ?? 0;

  const loadState = useCallback(() => Promise.all([
    invoke<OutlinePlan>("local_research_outline", { projectId }),
    invoke<SectionDraft[]>("list_local_research_sections", { projectId }),
    invoke<LedgerRow[]>("local_research_claim_ledger", { projectId }),
    invoke<EvidenceInventory>("local_research_evidence_facts", { projectId }),
    invoke<FigureAsset[]>("list_local_research_figure_assets", { projectId }),
  ]), [projectId]);
  const applyState = useCallback((result: [OutlinePlan, SectionDraft[], LedgerRow[], EvidenceInventory, FigureAsset[]]) => {
    const [plan, drafts, rows, evidence, figureAssets] = result;
    setOutline(plan);
    setSections(drafts);
    setLedger(rows);
    setInventory(evidence);
    setFigures(figureAssets);
  }, []);

  const loadFigurePreview = useCallback(async (figureId: string) => {
    try {
      const url = await invoke<string>("local_research_figure_data_url", { projectId, assetId: figureId });
      setFigurePreviews((current) => ({ ...current, [figureId]: url }));
    } catch {
      setFigurePreviews((current) => ({ ...current, [figureId]: "" }));
    }
  }, [projectId]);
  const refresh = useCallback(async () => {
    try {
      applyState(await loadState());
    } catch (caught) {
      setError(explain(caught));
    }
  }, [loadState, applyState]);

  useEffect(() => {
    let alive = true;
    loadState()
      .then((result) => { if (alive) applyState(result); })
      .catch((caught: unknown) => { if (alive) setError(explain(caught)); });
    return () => { alive = false; };
  }, [loadState, applyState]);

  useEffect(() => {
    const unlisten = listen<{ projectId: string; sectionKey: string; status: string }>("research:section-progress", (event) => {
      if (event.payload.projectId !== projectId) return;
      if (["running", "retrying"].includes(event.payload.status)) {
        setRunningKey(event.payload.sectionKey);
      } else {
        setRunningKey(null);
        void refresh();
      }
    });
    return () => { void unlisten.then((off) => off()); };
  }, [projectId, refresh]);

  const latestBySection = useMemo(() => {
    const map = new Map<string, SectionDraft>();
    for (const draft of sections) {
      const current = map.get(draft.sectionKey);
      if (!current || draft.revision > current.revision) map.set(draft.sectionKey, draft);
    }
    return map;
  }, [sections]);
  const approvedBySection = useMemo(() => {
    const map = new Map<string, SectionDraft>();
    for (const draft of sections) if (draft.status === "approved") map.set(draft.sectionKey, draft);
    return map;
  }, [sections]);

  async function classify(documentId: string, sourceClass: string) {
    setError(null);
    setBusyKey(`classify:${documentId}`);
    try {
      await invoke("classify_local_research_document", { projectId, documentId, sourceClass });
      await refresh();
    } catch (caught) {
      setError(explain(caught));
    } finally {
      setBusyKey(null);
    }
  }

  async function generate(sectionKey: string) {
    setError(null);
    setNotice(null);
    setRunningKey(sectionKey);
    setBusyKey(`generate:${sectionKey}`);
    try {
      const draft = await invoke<SectionDraft>("generate_local_research_section", { projectId, sectionKey, modelId: selectedModelId || null });
      if (draft.status === "insufficient_evidence") {
        setNotice(`${draft.heading}: ${draft.error ?? "insufficient evidence"}`);
      } else if (draft.status === "draft") {
        setNotice(`${draft.heading} drafted (revision ${draft.revision}). Review and approve before export.`);
      }
      await refresh();
    } catch (caught) {
      setError(explain(caught));
    } finally {
      setBusyKey(null);
      setRunningKey(null);
    }
  }

  async function cancelRunning() {
    try {
      await invoke("cancel_local_research_section", { projectId });
    } catch (caught) {
      setError(explain(caught));
    }
  }

  async function approve(draft: SectionDraft) {
    if (!draft.content || !draft.contentSha256) return;
    setError(null);
    setBusyKey(`approve:${draft.id}`);
    try {
      // The backend re-validates this token against the stored and recomputed
      // hash, so a stale draft is rejected rather than silently approved.
      await invoke("approve_local_research_section", { draftId: draft.id, expectedContentSha256: draft.contentSha256 });
      setNotice(`${draft.heading} approved.`);
      await refresh();
    } catch (caught) {
      setError(explain(caught));
    } finally {
      setBusyKey(null);
    }
  }

  async function saveEdit(draft: SectionDraft) {
    setError(null);
    setBusyKey(`edit:${draft.id}`);
    try {
      const saved = await invoke<SectionDraft>("save_local_research_section_edit", { projectId, sectionKey: draft.sectionKey, baseDraftId: draft.id, content: editDraftText });
      const unsupported = (saved.validation?.unsupportedTokens as unknown[] | undefined) ?? [];
      setNotice(unsupported.length ? `Saved revision ${saved.revision}. ${unsupported.length} number(s) are not linked to evidence and must be fixed before approval: ${unsupported.join(", ")}.` : `Saved revision ${saved.revision} as a new human-edited draft.`);
      setEditingDraftId(null);
      setPickerOpen(false);
      await refresh();
    } catch (caught) {
      setError(explain(caught));
    } finally {
      setBusyKey(null);
    }
  }

  function openEditor(draft: SectionDraft) {
    setEditingDraftId(draft.id);
    setEditDraftText(draft.content ?? "");
    setPickerOpen(false);
    setPickerReplaceIndex(null);
  }

  function insertSelectedFigure(asset: FigureAsset) {
    setEditDraftText((current) => {
      if (pickerReplaceIndex !== null) {
        return replaceFigureOccurrence(current, pickerReplaceIndex, asset);
      }
      const cursor = editTextareaRef.current?.selectionStart ?? current.length;
      return insertFigureAt(current, cursor, asset);
    });
    setPickerOpen(false);
    setPickerReplaceIndex(null);
    setNotice(pickerReplaceIndex !== null ? "Figure replaced. Save as a new revision, then re-approve." : "Figure inserted. Save as a new revision, then re-approve.");
  }

  const editDraftFigures = useMemo(() => parseFigureRefs(editDraftText), [editDraftText]);

  async function exportManuscript(format: "pdf" | "docx", allowDraft: boolean) {
    setError(null);
    setNotice(null);
    setBusyKey(`export:${format}`);
    try {
      const result = await invoke<ExportResult>("export_local_research_section_manuscript", { projectId, format, allowDraft });
      setExportResult(result);
      setNotice(`Exported ${format.toUpperCase()}: ${result.wordCount} words.`);
    } catch (caught) {
      setError(explain(caught));
    } finally {
      setBusyKey(null);
    }
  }

  const documents = inventory?.documents ?? [];
  const approvedCount = approvedBySection.size;

  return (
    <section className="rs-card rs-manuscript-workbench" aria-label="Evidence-grounded manuscript">
      <span className="rs-eyebrow">Evidence-grounded manuscript</span>
      <h2>Section-wise generation with a verifiable claim ledger</h2>
      <p>Numbers are extracted from classified evidence in Rust; the local model may only reference them. Nothing is padded to reach a page target, and every numeric statement is recorded in the claim-to-source ledger.</p>

      {!modelAvailable ? (
        <p className="rs-notice" role="status">Section generation is BLOCKED until a compatible installed local chat model is selected. Classification, human editing, and the evidence-limited export remain available. <button type="button" onClick={onOpenModelManager}>Open Model Manager</button></p>
      ) : null}

      <details className="rs-evidence-classes" open>
        <summary>Evidence classification ({numericFactCount} numeric fact{numericFactCount === 1 ? "" : "s"} available)</summary>
        <p>Classify each file. Only files classed as raw experiment or derived analysis provide numbers to the manuscript. <button type="button" className="rs-refresh-inline" disabled={Boolean(busyKey)} onClick={() => void refresh()}>Refresh evidence</button></p>
        {documents.length ? (
          <ul className="rs-local-sources">
            {documents.map((document) => (
              <li key={document.id}>
                <strong>{document.name}</strong>
                <small>{document.mediaType} · SHA-256 {document.sha256.slice(0, 16)}… · {document.factCount} numeric fact(s)</small>
                <label>Source class
                  <select value={document.sourceClass ?? ""} disabled={busyKey === `classify:${document.id}`} onChange={(event) => void classify(document.id, event.target.value)}>
                    <option value="" disabled>Choose classification…</option>
                    {SOURCE_CLASSES.map((item) => <option key={item.value} value={item.value}>{item.label}{item.numeric ? " (numeric)" : ""}</option>)}
                  </select>
                </label>
              </li>
            ))}
          </ul>
        ) : <small>No evidence imported to this project yet.</small>}
      </details>

      {outline ? (
        <div className="rs-outline">
          <p><strong>Planned allocation:</strong> {outline.plannedTotalWords.toLocaleString()} words across {outline.entries.length} sections for a {outline.pageTarget}-page target (assumes {outline.wordsPerPageAssumption} words/page). {outline.notice}</p>
          <table className="rs-section-table">
            <thead><tr><th>Section</th><th>Plan</th><th>Evidence</th><th>Status</th><th>Actions</th></tr></thead>
            <tbody>
              {outline.entries.map((entry) => {
                const latest = latestBySection.get(entry.key);
                const approved = approvedBySection.get(entry.key);
                const isRunning = runningKey === entry.key;
                return (
                  <tr key={entry.key}>
                    <td>{entry.heading}</td>
                    <td>{entry.plannedWords} w · {entry.plannedPages}p</td>
                    <td>{entry.evidenceRequirement}</td>
                    <td>
                      {approved ? <span className="rs-badge ok">approved r{approved.revision}</span> : latest ? <span className={`rs-badge ${latest.status === "draft" ? "draft" : latest.status === "insufficient_evidence" ? "blocked" : "warn"}`}>{latest.status.replace(/_/g, " ")}{latest.revision ? ` r${latest.revision}` : ""}</span> : <span className="rs-badge">not started</span>}
                      {latest?.error ? <small className="rs-error">{latest.error}</small> : null}
                    </td>
                    <td className="rs-section-actions">
                      <button type="button" disabled={!modelAvailable || Boolean(busyKey) || Boolean(runningKey)} onClick={() => void generate(entry.key)}>{latest ? "Regenerate" : "Generate"}</button>
                      {isRunning ? <button type="button" onClick={() => void cancelRunning()}>Cancel</button> : null}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      ) : null}

      {sections.filter((draft) => draft.status === "draft" || draft.status === "approved").length ? (
        <div className="rs-drafts">
          <h3>Section drafts</h3>
          {Array.from(latestBySection.values()).filter((draft) => draft.content).sort((left, right) => left.sectionKey.localeCompare(right.sectionKey)).map((draft) => {
            const warnings = (draft.validation?.warnings as string[] | undefined) ?? [];
            const unsupported = (draft.validation?.unsupportedTokens as unknown[] | undefined) ?? [];
            const editing = editingDraftId === draft.id;
            return (
              <article key={draft.id} className="rs-draft">
                <header><strong>{draft.heading}</strong> <small>revision {draft.revision} · {draft.origin} · {draft.status}{draft.modelId ? ` · ${draft.modelId}` : ""}{draft.latencyMs ? ` · ${(draft.latencyMs / 1000).toFixed(1)}s` : ""}</small></header>
                {editing ? (
                  <>
                    <textarea ref={editTextareaRef} rows={10} value={editDraftText} onChange={(event) => setEditDraftText(event.target.value)} aria-label={`Edit ${draft.heading}`} />
                    <div className="rs-actions">
                      <button type="button" disabled={busyKey === `edit:${draft.id}`} onClick={() => void saveEdit(draft)}>Save as new revision</button>
                      <button type="button" onClick={() => { setPickerReplaceIndex(null); setFigureQuery(""); setPickerOpen((open) => !open); }} aria-expanded={pickerOpen}>Insert figure</button>
                      <button type="button" onClick={() => { setEditingDraftId(null); setPickerOpen(false); }}>Cancel</button>
                    </div>
                    {editDraftFigures.length ? (
                      <div className="rs-figure-inline" aria-label="Figures in this section">
                        <strong>Figures in this section ({editDraftFigures.length})</strong>
                        <ol>
                          {editDraftFigures.map((ref, index) => {
                            const asset = figures.find((item) => item.id === ref.id);
                            return (
                              <li key={`${ref.id}-${index}`}>
                                <span>Figure {index + 1}: {asset ? asset.name : <em className="rs-error">unknown asset {ref.id.slice(0, 8)}…</em>} — {ref.caption || "(no caption)"}</span>
                                <span className="rs-figure-ops">
                                  <button type="button" disabled={index === 0} aria-label={`Move figure ${index + 1} up`} onClick={() => setEditDraftText((current) => moveFigureOccurrence(current, index, -1))}>↑</button>
                                  <button type="button" disabled={index === editDraftFigures.length - 1} aria-label={`Move figure ${index + 1} down`} onClick={() => setEditDraftText((current) => moveFigureOccurrence(current, index, 1))}>↓</button>
                                  <button type="button" aria-label={`Replace figure ${index + 1}`} onClick={() => { setPickerReplaceIndex(index); setFigureQuery(""); setPickerOpen(true); }}>Replace</button>
                                  <button type="button" aria-label={`Remove figure ${index + 1}`} onClick={() => setEditDraftText((current) => removeFigureOccurrence(current, index))}>Remove</button>
                                </span>
                              </li>
                            );
                          })}
                        </ol>
                      </div>
                    ) : null}
                    {pickerOpen ? (
                      <div className="rs-figure-picker" role="dialog" aria-label="Insert a project figure">
                        <div className="rs-figure-picker-head">
                          <strong>{pickerReplaceIndex !== null ? `Replace figure ${pickerReplaceIndex + 1}` : "Insert a project figure"}</strong>
                          <input type="search" aria-label="Search figures" placeholder="Search by name, caption or alt text…" value={figureQuery} onChange={(event) => setFigureQuery(event.target.value)} />
                          <button type="button" onClick={() => { setPickerOpen(false); setPickerReplaceIndex(null); }}>Close</button>
                        </div>
                        {figures.length === 0 ? <small>No project figures are registered. Register one in the project figure assets panel first.</small> : null}
                        <ul className="rs-figure-grid">
                          {figures.filter((asset) => { const q = figureQuery.trim().toLowerCase(); return !q || `${asset.name} ${asset.caption} ${asset.altText}`.toLowerCase().includes(q); }).map((asset) => {
                            const preview = figurePreviews[asset.id];
                            if (preview === undefined) void loadFigurePreview(asset.id);
                            return (
                              <li key={asset.id} className="rs-figure-card">
                                <div className="rs-figure-thumb">{preview ? <img src={preview} alt={asset.altText} /> : <span className="rs-figure-thumb-fallback">{preview === "" ? "preview unavailable" : "loading…"}</span>}</div>
                                <div className="rs-figure-meta">
                                  <strong>{asset.name}</strong>
                                  <small>{asset.caption}</small>
                                  <small>alt: {asset.altText}</small>
                                  <small>{asset.widthPx || "?"}×{asset.heightPx || "?"} · {asset.mediaType} · SHA-256 {asset.contentSha256.slice(0, 12)}…</small>
                                  <button type="button" onClick={() => insertSelectedFigure(asset)}>{pickerReplaceIndex !== null ? "Use this figure" : "Insert into section"}</button>
                                </div>
                              </li>
                            );
                          })}
                        </ul>
                      </div>
                    ) : null}
                  </>
                ) : (
                  <>
                    <pre className="rs-draft-body">{draft.content}</pre>
                    {warnings.map((warning) => <small key={warning} className="rs-warn" role="status">⚠ {warning}</small>)}
                    {unsupported.length ? <small className="rs-error" role="alert">Numbers not linked to evidence: {unsupported.join(", ")}. Edit before approval.</small> : null}
                    <div className="rs-actions">
                      {draft.status === "draft" ? <button type="button" disabled={busyKey === `approve:${draft.id}` || unsupported.length > 0} onClick={() => void approve(draft)}>Approve section</button> : null}
                      <button type="button" onClick={() => openEditor(draft)}>Edit</button>
                    </div>
                  </>
                )}
              </article>
            );
          })}
        </div>
      ) : null}

      <div className="rs-ledger">
        <button type="button" onClick={() => setShowLedger((value) => !value)}>{showLedger ? "Hide" : "Show"} claim-to-source ledger ({ledger.length})</button>
        {showLedger ? (
          ledger.length ? (
            <table className="rs-section-table">
              <thead><tr><th>Section</th><th>Value</th><th>Fact</th><th>Source (SHA-256)</th><th>Locator</th><th>Method</th></tr></thead>
              <tbody>
                {ledger.map((row, index) => (
                  <tr key={`${row.factId}-${index}`}>
                    <td>{row.sectionKey}{row.draftStatus === "approved" ? " ✓" : ""}</td>
                    <td>{row.renderedToken ?? "(qualitative)"}</td>
                    <td>{row.factId}</td>
                    <td title={row.documentSha256}>{row.documentName} ({row.documentSha256.slice(0, 12)})</td>
                    <td>{row.locator}</td>
                    <td>{row.method}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          ) : <small>No fact-linked claims recorded yet.</small>
        ) : null}
      </div>

      <div className="rs-export">
        <h3>Export</h3>
        <p>{approvedCount} approved section(s). PDF page count is measured after export and compared to the {outline?.pageTarget ?? "?"}-page target; a shortfall is reported honestly, never padded.</p>
        <div className="rs-actions">
          <button type="button" disabled={Boolean(busyKey) || approvedCount === 0} onClick={() => void exportManuscript("pdf", false)}>Export approved PDF</button>
          <button type="button" disabled={Boolean(busyKey) || approvedCount === 0} onClick={() => void exportManuscript("docx", false)}>Export approved DOCX</button>
          <button type="button" disabled={Boolean(busyKey)} onClick={() => void exportManuscript("pdf", true)}>Export draft PDF</button>
        </div>
        {exportResult ? (
          <div className="rs-export-result" role="status">
            <p><strong>{exportResult.format.toUpperCase()}</strong> · {exportResult.wordCount} words · {exportResult.draft ? "DRAFT" : "approved"}</p>
            <p>Page target: {exportResult.pageTarget} · Actual pages: {exportResult.actualPages ?? "not measured"} · Status: <strong className={exportResult.pageTargetStatus === "TARGET_MET" ? "rs-badge ok" : exportResult.pageTargetStatus === "INSUFFICIENT_EVIDENCE" ? "rs-badge blocked" : "rs-badge"}>{exportResult.pageTargetStatus.replace(/_/g, " ")}</strong></p>
            <small>Content SHA-256: {exportResult.contentSha256.slice(0, 24)}… · Ledger SHA-256: {exportResult.ledgerSha256.slice(0, 24)}…{exportResult.fileSha256 ? ` · File SHA-256: ${exportResult.fileSha256.slice(0, 24)}…` : ""}</small>
            {exportResult.pageTargetStatus === "INSUFFICIENT_EVIDENCE" ? <small className="rs-warn">The evidence supports fewer pages than the target. Add independent experiments rather than padding content.</small> : null}
            {exportResult.missingSections.length ? <small>Not included: {exportResult.missingSections.map((item) => String(item.heading)).join(", ")}.</small> : null}
          </div>
        ) : null}
      </div>

      {error ? <p className="rs-error" role="alert">{error}</p> : null}
      {notice ? <p className="rs-notice" role="status">{notice}</p> : null}
    </section>
  );
}
