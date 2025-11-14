interface ConnectionContextMenuProps {
  open: boolean;
  position: { x: number; y: number } | null;
  onConnect: () => void;
  onEdit: () => void;
  onDelete: () => void;
}

export default function ConnectionContextMenu({ open, position, onConnect, onEdit, onDelete }: ConnectionContextMenuProps) {
  if (!open || !position) return null;

  return (
    <div
      className="connection-menu"
      style={{
        position: "fixed",
        left: position.x,
        top: position.y,
      }}
    >
      <button
        type="button"
        className="connection-menu__item"
        onMouseDown={(event) => {
          event.preventDefault();
          onConnect();
        }}
      >
        Connect
      </button>
      <button
        type="button"
        className="connection-menu__item"
        onMouseDown={(event) => {
          event.preventDefault();
          onEdit();
        }}
      >
        Edit…
      </button>
      <button
        type="button"
        className="connection-menu__item connection-menu__item--danger"
        onMouseDown={(event) => {
          event.preventDefault();
          onDelete();
        }}
      >
        Delete
      </button>
    </div>
  );
}
