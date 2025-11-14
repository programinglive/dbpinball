import { useRef, useEffect } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Dialog, DialogContent, DialogDescription, DialogOverlay, DialogPortal, DialogTitle } from "@/components/ui/dialog";
import { X } from "lucide-react";

interface DbConnection {
  id: string;
  name: string;
  engine: "mysql" | "postgres";
  host: string;
  port: number;
  database: string;
  username: string;
  useSsl: boolean;
}

interface ConnectionPickerProps {
  open: boolean;
  query: string;
  onQueryChange: (value: string) => void;
  selectedId: string | null;
  onSelect: (conn: DbConnection) => void;
  onConfirm: (conn?: DbConnection) => void;
  onClose: () => void;
  onNew: () => void;
  onKeyDown: (event: React.KeyboardEvent<HTMLInputElement>) => void;
  connections: DbConnection[];
}

export default function ConnectionPicker(props: ConnectionPickerProps) {
  const {
    open,
    query,
    onQueryChange,
    selectedId,
    onSelect,
    onConfirm,
    onClose,
    onNew,
    onKeyDown,
    connections,
  } = props;

  const searchRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const frame = requestAnimationFrame(() => {
      searchRef.current?.focus();
    });
    return () => cancelAnimationFrame(frame);
  }, [open]);

  return (
    <Dialog open={open}>
      <DialogPortal>
        <DialogOverlay
          className="connection-picker__overlay"
          style={{ zIndex: 4000 }}
        />
        <DialogContent
          className="connection-picker"
          style={{ zIndex: 4100, width: "460px", maxWidth: "460px" }}
          hideClose
          onEscapeKeyDown={(event) => {
            event.preventDefault();
            onClose();
          }}
          onPointerDownOutside={(event) => {
            event.preventDefault();
            onClose();
          }}
        >
          <div className="connection-picker__titlebar">
            <DialogTitle className="connection-picker__title-text">Open connection</DialogTitle>
            <div className="connection-picker__titlebar-actions">
              <button
                type="button"
                className="connection-picker__titlebar-btn connection-picker__titlebar-btn--close"
                aria-label="Close connection picker"
                onClick={onClose}
                data-tauri-drag-region="false"
              >
                <X size={12} />
              </button>
            </div>
          </div>
          <DialogDescription className="sr-only">
            Search and pick from saved database connections. Use arrow keys to move and Enter to connect.
          </DialogDescription>

          <div className="connection-picker__body">
            <div className="connection-picker__search">
              <Input
                ref={searchRef}
                placeholder="Search for connection… (Ctrl F)"
                value={query}
                onChange={(event) => onQueryChange(event.target.value)}
                onKeyDown={onKeyDown}
                className="connection-picker__search-input"
                aria-label="Search connections"
              />
            </div>

            <div className="connection-picker__list" role="listbox" aria-label="Connections">
              {connections.length === 0 ? (
                <div className="connection-picker__empty">No matching connections.</div>
              ) : (
                <ul>
                  {connections.map((conn) => {
                    const isSelected = selectedId === conn.id;
                    const badge = conn.engine === "postgres" ? "Pg" : "My";
                    const description = conn.database ? `${conn.host} · ${conn.database}` : conn.host;
                    const statusChip = conn.engine === "postgres" ? "(PostgreSQL)" : "(MySQL)";

                    return (
                      <li key={conn.id}>
                        <button
                          type="button"
                          role="option"
                          aria-selected={isSelected}
                          className={`connection-picker__item${isSelected ? " connection-picker__item--active" : ""}`}
                          onMouseEnter={() => onSelect(conn)}
                          onFocus={() => onSelect(conn)}
                          onClick={() => onConfirm(conn)}
                        >
                          <span className="connection-picker__item-badge">{badge}</span>
                          <span className="connection-picker__item-content">
                            <span className="connection-picker__item-title">
                              {conn.name} <span className="connection-picker__item-tag">{statusChip}</span>
                            </span>
                            <span className="connection-picker__item-description">{description}</span>
                          </span>
                        </button>
                      </li>
                    );
                  })}
                </ul>
              )}
            </div>
          </div>

          <div className="connection-picker__footer">
            <Button variant="ghost" onClick={onNew} className="connection-picker__footer-btn">
              New…
            </Button>
            <Button
              onClick={() => onConfirm()}
              disabled={connections.length === 0}
              className="connection-picker__footer-btn connection-picker__footer-btn--primary"
            >
              Open
            </Button>
          </div>
        </DialogContent>
      </DialogPortal>
    </Dialog>
  );
}
