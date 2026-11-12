import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { Project, ProjectSummary } from "../types";
import { formatError } from "./format";

/**
 * A single bounded catalog query plus one full detail query for the selected
 * project. The catalog never includes project instructions or file bodies.
 */
export function useProjectCatalog() {
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [loadedDetail, setLoadedDetail] = useState<Project | null>(null);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [detailLoading, setDetailLoading] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [catalogError, setCatalogError] = useState<string | null>(null);
  const [detailError, setDetailError] = useState<string | null>(null);
  const [detailRevision, setDetailRevision] = useState(0);
  const catalogVersion = useRef(0);
  const detailVersion = useRef(0);
  const selectedIdRef = useRef(activeId);
  selectedIdRef.current = activeId;
  const loadMoreInFlight = useRef(false);

  const refreshProjects = useCallback(async (preferredId?: string | null) => {
    const version = ++catalogVersion.current;
    const page = await api.projects();
    if (catalogVersion.current !== version) return page.projects;
    // A selected project from an older loaded page must remain visible after
    // a mutation refresh resets the listing to the first page. Otherwise its
    // select/side-menu option vanishes while its detail is still open.
    const selectedId = preferredId === null ? null : preferredId ?? selectedIdRef.current;
    setProjects((current) => {
      const retained = selectedId && !page.projects.some((project) => project.id === selectedId)
        ? current.find((project) => project.id === selectedId)
        : null;
      return retained ? [retained, ...page.projects] : page.projects;
    });
    setNextCursor(page.nextCursor);
    setCatalogError(null);
    setActiveId((current) => {
      if (preferredId === null) return page.projects[0]?.id ?? null;
      const candidate = preferredId ?? current;
      // A selected project can be outside the first catalog page. The detail
      // lookup remains valid, and pagination can bring it back into the list.
      return candidate ?? page.projects[0]?.id ?? null;
    });
    setDetailRevision((revision) => revision + 1);
    return page.projects;
  }, []);

  const loadMore = useCallback(async () => {
    if (!nextCursor || loadMoreInFlight.current) return;
    loadMoreInFlight.current = true;
    setLoadingMore(true);
    const version = catalogVersion.current;
    try {
      const page = await api.projects(nextCursor);
      if (version !== catalogVersion.current) return;
      setProjects((current) => {
        const seen = new Set(current.map((project) => project.id));
        return [...current, ...page.projects.filter((project) => !seen.has(project.id))];
      });
      setNextCursor(page.nextCursor);
      setCatalogError(null);
    } catch (error) {
      if (version === catalogVersion.current) setCatalogError(formatError(error));
    } finally {
      loadMoreInFlight.current = false;
      setLoadingMore(false);
    }
  }, [nextCursor]);

  useEffect(() => {
    let alive = true;
    void refreshProjects()
      .catch((error) => {
        if (alive) setCatalogError(formatError(error));
      })
      .finally(() => {
        if (alive) setLoading(false);
      });
    return () => {
      alive = false;
      catalogVersion.current += 1;
    };
  }, [refreshProjects]);

  useEffect(() => {
    const version = ++detailVersion.current;
    if (!activeId) {
      setLoadedDetail(null);
      setDetailLoading(false);
      setDetailError(null);
      return;
    }
    setDetailLoading(true);
    setDetailError(null);
    // Never display the previous project's text during a fast selection.
    setLoadedDetail(null);
    void api.getProject(activeId)
      .then((project) => {
        if (version === detailVersion.current) setLoadedDetail(project);
      })
      .catch((error) => {
        if (version === detailVersion.current) setDetailError(formatError(error));
      })
      .finally(() => {
        if (version === detailVersion.current) setDetailLoading(false);
      });
    return () => {
      detailVersion.current += 1;
    };
  }, [activeId, detailRevision]);

  // A selected ID change takes effect immediately, before effect cleanup.
  const activeProject = loadedDetail?.id === activeId ? loadedDetail : null;
  return {
    projects, setProjects, activeId, setActiveId, activeProject,
    setActiveProject: setLoadedDetail, loading, detailLoading,
    loadingMore, nextCursor, loadMore, refreshProjects,
    catalogError, detailError,
  };
}
