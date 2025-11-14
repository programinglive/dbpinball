import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { MouseEvent as ReactMouseEvent } from "react";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

type QueryResultRow = (string | null)[];

interface QueryResultPayload {
  columns: string[];
  rows: QueryResultRow[];
}

interface ResultGridProps {
  result: QueryResultPayload | null;
  running: boolean;
  error: string | null;
  tableName?: string | null;
}

type ResultViewMode = "data" | "structure" | "raw";

const MIN_COLUMN_WIDTH = 80;
const DEFAULT_COLUMN_WIDTH = 160;

type ResizingState =
  | { kind: "single"; column: string; startX: number; startWidth: number }
  | {
      kind: "dual";
      leftColumn: string;
      rightColumn: string;
      startX: number;
      leftWidth: number;
      rightWidth: number;
    };

export default function ResultGrid({ result, running, error, tableName }: ResultGridProps) {
  const [viewMode, setViewMode] = useState<ResultViewMode>("data");
  const [columnWidths, setColumnWidths] = useState<Record<string, number>>({});
  const resizingRef = useRef<ResizingState | null>(null);
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const hscrollRef = useRef<HTMLDivElement | null>(null);
  const columns = useMemo(() => result?.columns ?? [], [result?.columns]);

  const hasResult = !!result && result.columns.length > 0;

  useEffect(() => {
    if (!columns.length) {
      return;
    }

    setColumnWidths((prev) => {
      let next = prev;
      columns.forEach((column) => {
        if (!(column in next)) {
          if (next === prev) {
            next = { ...prev };
          }

          next[column] = DEFAULT_COLUMN_WIDTH;
        }
      });

      return next;
    });
  }, [columns]);

  // Dedicated horizontal scrollbar below the grid controls the table scrollLeft.
  useEffect(() => {
    const viewport = viewportRef.current;
    const hscroll = hscrollRef.current;
    if (!viewport || !hscroll) {
      return;
    }

    const updateHelperWidth = () => {
      const inner = hscroll.firstElementChild as HTMLDivElement | null;
      if (!inner || !viewport) return;

      const table = viewport.querySelector('.workspace-panel__result-table') as HTMLElement | null;
      const clientWidth = viewport.clientWidth;
      const tableScrollWidth = table?.scrollWidth ?? clientWidth;

      // Make helper track match the table's scrollable width
      const helperWidth = Math.max(tableScrollWidth, clientWidth + 100);
      inner.style.width = `${helperWidth}px`;

      setTimeout(() => {
        console.log('[Helper] Updated width:', {
          clientWidth,
          helperWidth,
          innerWidth: inner.style.width,
          hscrollScrollWidth: hscroll.scrollWidth,
          hscrollClientWidth: hscroll.clientWidth,
          hscrollMaxScroll: hscroll.scrollWidth - hscroll.clientWidth,
          tableWidth: table?.offsetWidth,
          tableScrollWidth,
          columnCount: columns.length,
          totalColumnWidth: columns.reduce((sum, col) => sum + (columnWidths[col] || 160), 0)
        });
      }, 0);
    };

    updateHelperWidth();

    // Update helper width when viewport content changes
    const resizeObserver = new ResizeObserver(() => {
      updateHelperWidth();
    });
    resizeObserver.observe(viewport);

    const handleHelperScroll = () => {
      if (!viewport || !hscroll) {
        console.log('[Helper] Missing refs:', { viewport: !!viewport, hscroll: !!hscroll });
        return;
      }

      const table = viewport.querySelector('.workspace-panel__result-table') as HTMLElement | null;
      const tableScrollWidth = table?.scrollWidth ?? viewport.scrollWidth;
      const maxViewportScroll = tableScrollWidth - viewport.clientWidth;
      const maxHelperScroll = hscroll.scrollWidth - hscroll.clientWidth;

      console.log('[Helper] Scroll event:', {
        helperScrollLeft: hscroll.scrollLeft,
        maxViewportScroll,
        maxHelperScroll,
        viewportScrollWidth: viewport.scrollWidth,
        viewportClientWidth: viewport.clientWidth,
        hscrollScrollWidth: hscroll.scrollWidth,
        hscrollClientWidth: hscroll.clientWidth,
        tableScrollWidth
      });

      if (maxViewportScroll <= 0 || maxHelperScroll <= 0) {
        console.log('[Helper] No scroll range available');
        return;
      }

      const ratio = hscroll.scrollLeft / maxHelperScroll;
      const newScrollLeft = ratio * maxViewportScroll;

      // Apply scroll position in two ways:
      // 1) Set viewport.scrollLeft (native scrolling if browser recognizes overflow)
      // 2) Translate the table as a fallback so content always moves visually
      viewport.scrollLeft = newScrollLeft;
      if (table) {
        table.style.transform = `translateX(${-newScrollLeft}px)`;
      }

      console.log('[Helper] Set viewport.scrollLeft:', newScrollLeft, 'ratio:', ratio);
    };

    hscroll.addEventListener("scroll", handleHelperScroll, { passive: true });

    return () => {
      resizeObserver.disconnect();
      hscroll.removeEventListener("scroll", handleHelperScroll);
    };
  }, [columns, columnWidths, hasResult]);

  const handleMouseMove = useCallback((event: MouseEvent) => {
    const resizing = resizingRef.current;
    if (!resizing) {
      return;
    }

    if (resizing.kind === "single") {
      const delta = event.clientX - resizing.startX;
      const newWidth = Math.max(MIN_COLUMN_WIDTH, resizing.startWidth + delta);

      setColumnWidths((prev) => {
        const current = prev[resizing.column];
        if (current === newWidth) {
          return prev;
        }

        return {
          ...prev,
          [resizing.column]: newWidth,
        };
      });
      return;
    }

    const delta = event.clientX - resizing.startX;
    const minDelta = MIN_COLUMN_WIDTH - resizing.leftWidth;
    const maxDelta = resizing.rightWidth - MIN_COLUMN_WIDTH;
    const clampedDelta = Math.min(Math.max(delta, minDelta), maxDelta);
    const nextLeftWidth = resizing.leftWidth + clampedDelta;
    const nextRightWidth = resizing.rightWidth - clampedDelta;

    setColumnWidths((prev) => {
      const leftChanged = prev[resizing.leftColumn] !== nextLeftWidth;
      const rightChanged = prev[resizing.rightColumn] !== nextRightWidth;
      if (!leftChanged && !rightChanged) {
        return prev;
      }

      return {
        ...prev,
        [resizing.leftColumn]: nextLeftWidth,
        [resizing.rightColumn]: nextRightWidth,
      };
    });
  }, []);

  const handleMouseUp = useCallback(() => {
    resizingRef.current = null;
    window.removeEventListener("mousemove", handleMouseMove);
    window.removeEventListener("mouseup", handleMouseUp);
  }, [handleMouseMove]);

  const startSingleResize = useCallback(
    (column: string, event: ReactMouseEvent) => {
      event.preventDefault();
      event.stopPropagation();

      const currentWidth = columnWidths[column] ?? DEFAULT_COLUMN_WIDTH;

      resizingRef.current = {
        kind: "single",
        column,
        startX: event.clientX,
        startWidth: currentWidth,
      };

      window.addEventListener("mousemove", handleMouseMove);
      window.addEventListener("mouseup", handleMouseUp);
    },
    [columnWidths, handleMouseMove, handleMouseUp]
  );

  const startBoundaryResize = useCallback(
    (leftColumn: string, rightColumn: string, event: ReactMouseEvent) => {
      event.preventDefault();
      event.stopPropagation();

      const leftWidth = columnWidths[leftColumn] ?? DEFAULT_COLUMN_WIDTH;
      const rightWidth = columnWidths[rightColumn] ?? DEFAULT_COLUMN_WIDTH;

      resizingRef.current = {
        kind: "dual",
        leftColumn,
        rightColumn,
        startX: event.clientX,
        leftWidth,
        rightWidth,
      };

      window.addEventListener("mousemove", handleMouseMove);
      window.addEventListener("mouseup", handleMouseUp);
    },
    [columnWidths, handleMouseMove, handleMouseUp]
  );

  useEffect(() => {
    return () => {
      window.removeEventListener("mousemove", handleMouseMove);
      window.removeEventListener("mouseup", handleMouseUp);
    };
  }, [handleMouseMove, handleMouseUp]);

  let statusLabel: string;
  if (running) {
    statusLabel = "Running query…";
  } else if (error) {
    statusLabel = "Error";
  } else if (!hasResult) {
    statusLabel = "Ready";
  } else if (viewMode === "data") {
    statusLabel = `${result!.rows.length.toLocaleString()} rows`;
  } else if (viewMode === "structure") {
    statusLabel = `${result!.columns.length.toLocaleString()} columns`;
  } else {
    statusLabel = `${result!.rows.length.toLocaleString()} rows (raw)`;
  }

  let content;

  if (error) {
    content = (
      <div className="workspace-panel__result-empty">
        <p className="workspace-panel__error" role="alert">
          {error}
        </p>
      </div>
    );
  } else if (running && !result) {
    content = (
      <div className="workspace-panel__result-empty">
        <div className="workspace-panel__status">Running query…</div>
      </div>
    );
  } else if (!hasResult) {
    content = (
      <div className="workspace-panel__result-empty">
        <div className="workspace-panel__result-empty-box">
          <p className="workspace-panel__result-empty-title">No data loaded</p>
          <p className="workspace-panel__result-empty-text">Select a table or run a query to preview rows.</p>
        </div>
      </div>
    );
  } else {
    if (viewMode === "data") {
      content = (
        <div className="workspace-panel__result-scroll">
          <div className="workspace-panel__result-viewport" ref={viewportRef}>
            <Table className="workspace-panel__result-table">
              <TableHeader>
                <TableRow>
                  {columns.map((column, columnIndex) => {
                      const width = columnWidths[column];
                      const prevColumn = columns[columnIndex - 1];
                      const nextColumn = columns[columnIndex + 1];
                      return (
                        <TableHead
                          key={`${column}-${columnIndex}`}
                          style={width ? { width, minWidth: width } : undefined}
                        >
                          {prevColumn && (
                            <span
                              role="separator"
                              aria-label={`Resize boundary between ${prevColumn} and ${column}`}
                              className="workspace-panel__result-resizer workspace-panel__result-resizer--left"
                              onMouseDown={(event) => startBoundaryResize(prevColumn, column, event)}
                            />
                          )}
                          <div className="workspace-panel__result-th">
                            <span className="workspace-panel__result-th-label">{column}</span>
                          </div>
                          <span
                            role="separator"
                            aria-label={`Resize ${column}${nextColumn ? ` and ${nextColumn}` : ""}`}
                            className="workspace-panel__result-resizer workspace-panel__result-resizer--right"
                            onMouseDown={(event) =>
                              nextColumn
                                ? startBoundaryResize(column, nextColumn, event)
                                : startSingleResize(column, event)
                            }
                          />
                        </TableHead>
                      );
                    })}
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {result!.rows.map((row, rowIndex) => (
                    <TableRow key={rowIndex}>
                      {row.map((value, cellIndex) => {
                        const columnName = columns[cellIndex];
                        const width = columnWidths[columnName];
                        return (
                          <TableCell
                            key={cellIndex}
                            style={width ? { width, minWidth: width } : undefined}
                          >
                            {value === null ? (
                              <span className="workspace-panel__result-null">NULL</span>
                            ) : (
                              value
                            )}
                          </TableCell>
                        );
                      })}
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          <div className="workspace-panel__result-hscroll" ref={hscrollRef}>
            <div className="workspace-panel__result-hscroll-inner" />
          </div>
        </div>
      );
    } else if (viewMode === "structure") {
      content = (
        <div className="workspace-panel__result-scroll">
          <div className="workspace-panel__structure-list">
            {result!.columns.map((column, index) => (
              <div key={`${column}-${index}`} className="workspace-panel__structure-row">
                <span className="workspace-panel__structure-name">{column}</span>
              </div>
            ))}
          </div>
        </div>
      );
    } else {
      content = (
        <div className="workspace-panel__result-scroll">
          <pre className="workspace-panel__raw">
            {JSON.stringify({ columns: result!.columns, rows: result!.rows }, null, 2)}
          </pre>
        </div>
      );
    }
  }

  return (
    <>
      {tableName && (
        <div className="workspace-panel__infobar">
          <div className="workspace-panel__infobar-left">
            <span className="workspace-panel__infobar-table">{tableName}</span>
          </div>
          <div className="workspace-panel__infobar-right">
            <button type="button" className="workspace-panel__infobar-btn" disabled>
              Error
            </button>
            <button type="button" className="workspace-panel__infobar-btn" disabled>
              Filters
            </button>
            <button type="button" className="workspace-panel__infobar-btn" disabled>
              Columns
            </button>
            <span className="workspace-panel__infobar-sep" />
            <button type="button" className="workspace-panel__infobar-icon" disabled>
              ⟲
            </button>
            <button type="button" className="workspace-panel__infobar-icon" disabled>
              ☰
            </button>
          </div>
        </div>
      )}
      {content}
      <div className="workspace-panel__statusbar">
        <div
          className="workspace-panel__statusbar-left"
          aria-label="Result view mode"
          data-testid="result-view-toggle"
        >
          <button
            type="button"
            className={`workspace-panel__statusbar-seg${viewMode === "data" ? " workspace-panel__statusbar-seg--active" : ""}`}
            onClick={() => setViewMode("data")}
          >
            Data
          </button>
          <button
            type="button"
            className={`workspace-panel__statusbar-seg${viewMode === "structure" ? " workspace-panel__statusbar-seg--active" : ""}`}
            onClick={() => setViewMode("structure")}
          >
            Structure
          </button>
          <button
            type="button"
            className={`workspace-panel__statusbar-seg${viewMode === "raw" ? " workspace-panel__statusbar-seg--active" : ""}`}
            onClick={() => setViewMode("raw")}
          >
            Raw
          </button>
        </div>
        <div className="workspace-panel__statusbar-center">{statusLabel}</div>
        <div className="workspace-panel__statusbar-right" data-testid="result-view-actions">
          <button type="button" className="workspace-panel__statusbar-btn">Filter</button>
          <button type="button" className="workspace-panel__statusbar-btn">Columns</button>
          <button type="button" className="workspace-panel__statusbar-nav" disabled>
            &#x2039;
          </button>
          <button type="button" className="workspace-panel__statusbar-nav" disabled>
            &#x203A;
          </button>
        </div>
      </div>
    </>
  );
}
