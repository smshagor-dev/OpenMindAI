import { BookOpenText, FilePenLine, Search, ShieldAlert } from "lucide-react";
import type { WebResearchProgressEvent } from "../types";
import "../research-progress.css";

const operations = [
  { id: "searching", label: "Find sources", Icon: Search },
  { id: "reading", label: "Read pages", Icon: BookOpenText },
  { id: "generating", label: "Write answer", Icon: FilePenLine },
] as const;

export function ResearchProgress({ progress }: { progress: WebResearchProgressEvent }) {
  const isStopped = progress.stage === "stopped" || progress.stage === "unavailable";
  const currentIndex = operations.findIndex((operation) => operation.id === progress.stage);
  const activeIndex = currentIndex < 0 ? 0 : currentIndex;
  const title = progress.mode === "research" ? "Deep Research" : "Web Search";
  const description =
    progress.stage === "searching"
      ? `Search queries completed: ${progress.completed} / ${progress.total}`
      : progress.stage === "reading"
        ? `Webpages checked: ${progress.completed} / ${progress.total}`
        : progress.stage === "generating"
          ? "Retrieval finished. Composing the answer locally."
          : progress.stage === "stopped"
            ? "Research stopped."
            : "Live retrieval could not be completed.";

  return (
    <section className="research-progress" role="status" aria-live="polite" aria-label={title + " activity"}>
      <div className="research-progress-heading">
        <strong>{title}</strong>
        <span>{isStopped ? "Retrieval interrupted" : "In progress"}</span>
      </div>
      <div className="research-progress-operations" aria-label="Completed research operations">
        {operations.map(({ id, label, Icon }, index) => {
          const isCurrent = !isStopped && id === progress.stage;
          const isFinished = !isStopped && index < activeIndex;
          return (
            <span
              key={id}
              className={`research-progress-operation${isCurrent ? " is-current" : ""}${isFinished ? " is-finished" : ""}`}
              aria-current={isCurrent ? "step" : undefined}
            >
              <Icon size={14} aria-hidden="true" />
              {label}
            </span>
          );
        })}
      </div>
      <p className="research-progress-description">
        {isStopped ? <ShieldAlert size={14} aria-hidden="true" /> : null}
        {description}
      </p>
      <div className="research-progress-counts">
        <span>Sources found: <strong>{progress.sources}</strong></span>
        <span>Pages read: <strong>{progress.pagesRead}</strong></span>
      </div>
    </section>
  );
}
