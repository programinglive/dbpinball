import { useRef, useEffect } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Dialog, DialogContent, DialogDescription, DialogOverlay, DialogPortal, DialogTitle } from "@/components/ui/dialog";
import { X } from "lucide-react";

interface DatabasePickerProps {
  open: boolean;
  query: string;
  onQueryChange: (value: string) => void;
  selected: string | null;
  onSelect: (dbName: string) => void;
  onConfirm: (dbName?: string) => void;
  onClose: () => void;
  onKeyDown: (event: React.KeyboardEvent<HTMLInputElement>) => void;
  databases: string[];
}

export default function DatabasePicker(props: DatabasePickerProps) {
  const {
    open,
    query,
    onQueryChange,
    selected,
    onSelect,
    onConfirm,
    onClose,
    onKeyDown,
    databases,
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
        <DialogOverlay className="database-picker__overlay" style={{ zIndex: 5000 }} />
        <DialogContent
          className="database-picker"
          style={{ zIndex: 5010, width: "420px", maxWidth: "420px" }}
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
          <div className="database-picker__titlebar">
            <DialogTitle className="database-picker__title-text">Open database</DialogTitle>
            <div className="database-picker__titlebar-actions">
              <button
                type="button"
                className="database-picker__titlebar-btn database-picker__titlebar-btn--close"
                aria-label="Close database picker"
                onClick={onClose}
              >
                <X size={12} />
              </button>
            </div>
          </div>
          <DialogDescription className="sr-only">
            Search and select a database to load its tables.
          </DialogDescription>

          <div className="database-picker__body">
            <Input
              ref={searchRef}
              placeholder="Search for database…"
              value={query}
              onChange={(event) => onQueryChange(event.target.value)}
              onKeyDown={onKeyDown}
              className="database-picker__search"
              aria-label="Search databases"
            />

            <div className="database-picker__list" role="listbox" aria-label="Databases">
              {databases.length === 0 ? (
                <div className="database-picker__empty">No databases found.</div>
              ) : (
                <ul>
                  {databases.map((dbName) => {
                    const isActive = selected === dbName;

                    return (
                      <li key={dbName}>
                        <button
                          type="button"
                          role="option"
                          aria-selected={isActive}
                          className={`database-picker__item${isActive ? " database-picker__item--active" : ""}`}
                          onMouseEnter={() => onSelect(dbName)}
                          onFocus={() => onSelect(dbName)}
                          onClick={() => onConfirm(dbName)}
                        >
                          <span className="database-picker__item-label">{dbName}</span>
                        </button>
                      </li>
                    );
                  })}
                </ul>
              )}
            </div>
          </div>

          <div className="database-picker__footer">
            <Button
              onClick={() => onConfirm()}
              disabled={databases.length === 0}
              className="database-picker__footer-btn database-picker__footer-btn--primary"
            >
              Open
            </Button>
          </div>
        </DialogContent>
      </DialogPortal>
    </Dialog>
  );
}
