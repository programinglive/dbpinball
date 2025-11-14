import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Button } from "@/components/ui/button";
import type { DbConnection } from "@/pages/connections.types";

interface ConnectionDialogProps {
  open: boolean;
  editingId: string | null;
  formData: Partial<DbConnection>;
  password: string;
  onOpenChange: (nextOpen: boolean) => void;
  onChangeField: (next: Partial<DbConnection>) => void;
  onChangePassword: (value: string) => void;
  onCancel: () => void;
  onSubmit: () => void;
}

export default function ConnectionDialog(props: ConnectionDialogProps) {
  const {
    open,
    editingId,
    formData,
    password,
    onOpenChange,
    onChangeField,
    onChangePassword,
    onCancel,
    onSubmit,
  } = props;

  const handleFieldChange = (patch: Partial<DbConnection>) => {
    onChangeField({ ...formData, ...patch });
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="connection-dialog">
        <DialogHeader className="connection-dialog__header">
          <DialogTitle className="connection-dialog__title">
            {editingId ? "Edit Database Connection" : "Add Database Connection"}
          </DialogTitle>
          <DialogDescription className="connection-dialog__description">
            {editingId ? "Update the connection details." : "Create a new database connection."}
          </DialogDescription>
        </DialogHeader>

        <div className="connection-form">
          <section className="connection-form__section">
            <header className="connection-form__section-header">
              <h3 className="connection-form__section-title">Connection</h3>
              <p className="connection-form__section-description">
                Describe the database and where it can be reached.
              </p>
            </header>

            <div className="connection-form__row">
              <div className="connection-field connection-field--grow">
                <Label htmlFor="name">Connection Name</Label>
                <Input
                  id="name"
                  placeholder="My Database"
                  value={formData.name || ""}
                  className="connection-input"
                  onChange={(e) => handleFieldChange({ name: e.target.value })}
                />
              </div>
              <div className="connection-field connection-field--slim">
                <Label htmlFor="engine">Database Engine</Label>
                <Select
                  value={formData.engine}
                  onValueChange={(v) => handleFieldChange({ engine: v as "mysql" | "postgres", port: v === "postgres" ? 5432 : 3306 })}
                >
                  <SelectTrigger className="connection-select">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="mysql">MySQL</SelectItem>
                    <SelectItem value="postgres">PostgreSQL</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </div>

            <div className="connection-form__row">
              <div className="connection-field connection-field--grow">
                <Label htmlFor="host">Host</Label>
                <Input
                  id="host"
                  placeholder="localhost"
                  value={formData.host || ""}
                  className="connection-input"
                  onChange={(e) => handleFieldChange({ host: e.target.value })}
                />
              </div>
              <div className="connection-field connection-field--compact">
                <Label htmlFor="port">Port</Label>
                <Input
                  id="port"
                  type="number"
                  value={formData.port ?? ""}
                  className="connection-input"
                  onChange={(e) => handleFieldChange({ port: parseInt(e.target.value) || 0 })}
                />
              </div>
            </div>

            <div className="connection-form__row connection-form__row--single">
              <div className="connection-field">
                <Label htmlFor="database">Database</Label>
                <Input
                  id="database"
                  placeholder="mydb"
                  value={formData.database || ""}
                  className="connection-input"
                  onChange={(e) => handleFieldChange({ database: e.target.value })}
                />
              </div>
            </div>
          </section>

          <section className="connection-form__section">
            <header className="connection-form__section-header">
              <h3 className="connection-form__section-title">Authentication</h3>
              <p className="connection-form__section-description">
                How we sign in to the database.
              </p>
            </header>

            <div className="connection-form__row">
              <div className="connection-field connection-field--grow">
                <Label htmlFor="username">Username</Label>
                <Input
                  id="username"
                  placeholder="root"
                  value={formData.username || ""}
                  className="connection-input"
                  onChange={(e) => handleFieldChange({ username: e.target.value })}
                />
              </div>
              <div className="connection-field connection-field--grow">
                <Label htmlFor="password">Password</Label>
                <Input
                  id="password"
                  type="password"
                  placeholder="••••••••"
                  value={password}
                  className="connection-input"
                  onChange={(e) => onChangePassword(e.target.value)}
                />
              </div>
            </div>
          </section>
        </div>

        <div className="connection-dialog__footer">
          <div className="connection-dialog__footer-left">
            <span className="connection-form__footer-hint">Save to persist changes or test the connection from the list.</span>
          </div>
          <div className="connection-dialog__footer-right">
            <Button
              type="button"
              variant="outline"
              className="connection-btn connection-btn--ghost"
              onClick={onCancel}
            >
              Cancel
            </Button>
            <Button type="button" onClick={onSubmit} className="connection-dialog__primary-btn">
              {editingId ? "Update Connection" : "Save Connection"}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
