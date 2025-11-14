import { useMemo } from "react";
import { Input } from "@/components/ui/input";
import { Database, ChevronRight, Search } from "lucide-react";

interface ItemsSidebarProps {
  query: string;
  onQueryChange: (value: string) => void;
  functionsOpen: boolean;
  onToggleFunctions: () => void;
  tablesOpen: boolean;
  onToggleTables: () => void;
  tablesLoading: boolean;
  tablesError: string | null;
  tables: string[];
  selectedTable: string | null;
  onSelectTable: (tableName: string) => void;
}

export default function ItemsSidebar(props: ItemsSidebarProps) {
  const {
    query,
    onQueryChange,
    functionsOpen,
    onToggleFunctions,
    tablesOpen,
    onToggleTables,
    tablesLoading,
    tablesError,
    tables,
    selectedTable,
    onSelectTable,
  } = props;

  const filteredTables = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    if (normalized.length === 0) return tables;
    return tables.filter((t) => t.toLowerCase().includes(normalized));
  }, [query, tables]);

  return (
    <div className="items-sidebar">
      <div className="items-sidebar__search">
        <Search size={14} aria-hidden="true" />
        <Input
          value={query}
          onChange={(event) => onQueryChange(event.target.value)}
          placeholder="Search items"
          className="items-sidebar__search-input"
        />
      </div>

      <div className="items-sidebar__scroll">
        <div className="items-sidebar__section">
          <button
            type="button"
            className="items-sidebar__section-header"
            onClick={onToggleFunctions}
          >
            <span>Functions</span>
            <ChevronRight
              size={14}
              className={`items-sidebar__section-icon${functionsOpen ? " items-sidebar__section-icon--open" : ""}`}
            />
          </button>

          {functionsOpen && (
            <div className="items-sidebar__section-body">
              <div className="items-sidebar__empty">
                <Database size={32} strokeWidth={1.3} aria-hidden="true" />
                <p>No functions yet.</p>
                <span>Create a function to see it here.</span>
              </div>
            </div>
          )}
        </div>

        <div className="items-sidebar__section">
          <button
            type="button"
            className="items-sidebar__section-header"
            onClick={onToggleTables}
          >
            <span>Tables</span>
            <ChevronRight
              size={14}
              className={`items-sidebar__section-icon${tablesOpen ? " items-sidebar__section-icon--open" : ""}`}
            />
          </button>

          {tablesOpen && (
            <div className="items-sidebar__section-body">
              {tablesError ? (
                <div className="items-sidebar__status items-sidebar__status--error">{tablesError}</div>
              ) : tablesLoading ? (
                <div className="items-sidebar__status">Loading tables…</div>
              ) : filteredTables.length === 0 ? (
                <div className="items-sidebar__empty">
                  <Database size={32} strokeWidth={1.3} aria-hidden="true" />
                  <p>No tables match your search.</p>
                  <span>Try a different search or refresh.</span>
                </div>
              ) : (
                <ul className="items-sidebar__list">
                  {filteredTables.map((tableName) => {
                    const isSelected = selectedTable === tableName;
                    return (
                      <li key={tableName}>
                        <button
                          type="button"
                          className={`items-sidebar__item${isSelected ? " items-sidebar__item--active" : ""}`}
                          onClick={() => onSelectTable(tableName)}
                        >
                          <span>{tableName}</span>
                        </button>
                      </li>
                    );
                  })}
                </ul>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
