import { useState } from "react";
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

export default function ResultGrid({ result, running, error, tableName }: ResultGridProps) {
  const [viewMode, setViewMode] = useState<ResultViewMode>("data");

  const hasResult = !!result && result.columns.length > 0;

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
          <Table className="workspace-panel__result-table">
            <TableHeader>
              <TableRow>
                {result!.columns.map((column, columnIndex) => (
                  <TableHead key={`${column}-${columnIndex}`}>{column}</TableHead>
                ))}
              </TableRow>
            </TableHeader>
            <TableBody>
              {result!.rows.map((row, rowIndex) => (
                <TableRow key={rowIndex}>
                  {row.map((value, cellIndex) => (
                    <TableCell key={cellIndex}>
                      {value === null ? (
                        <span className="workspace-panel__result-null">NULL</span>
                      ) : (
                        value
                      )}
                    </TableCell>
                  ))}
                </TableRow>
              ))}
            </TableBody>
          </Table>
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
