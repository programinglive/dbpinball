interface TableTabsProps {
  tabs: string[];
  active: string | null;
  onSelect: (name: string) => void;
  onClose: (name: string) => void;
}

export default function TableTabs({ tabs, active, onSelect, onClose }: TableTabsProps) {
  if (!tabs || tabs.length === 0) return null;

  return (
    <div className="workspace-panel__table-tabs">
      {tabs.map((name) => {
        const isActive = active === name;
        return (
          <button
            key={name}
            type="button"
            className={`workspace-panel__table-tab${isActive ? " workspace-panel__table-tab--active" : ""}`}
            onClick={() => onSelect(name)}
          >
            <span className="workspace-panel__table-tab-label">{name}</span>
            <span
              role="button"
              aria-label={`Close ${name}`}
              className="workspace-panel__table-tab-close"
              onClick={(event) => {
                event.stopPropagation();
                onClose(name);
              }}
            >
              ×
            </span>
          </button>
        );
      })}
    </div>
  );
}
