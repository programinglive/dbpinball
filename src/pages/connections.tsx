import { useState, useEffect } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle, DialogTrigger } from "@/components/ui/dialog";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Label } from "@/components/ui/label";
import { invoke } from "@tauri-apps/api/core";
import { Plus, Trash2, CheckCircle, AlertCircle } from "lucide-react";

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

export default function ConnectionsPage() {
  const [connections, setConnections] = useState<DbConnection[]>([]);
  const [open, setOpen] = useState(false);
  const [testing, setTesting] = useState<string | null>(null);
  const [testResult, setTestResult] = useState<Record<string, boolean>>({});
  const [formData, setFormData] = useState<Partial<DbConnection>>({
    engine: "mysql",
    port: 3306,
    useSsl: false,
  });
  const [password, setPassword] = useState("");

  useEffect(() => {
    loadConnections();
  }, []);

  const loadConnections = async () => {
    try {
      const stored = localStorage.getItem("dbpinball_connections");
      if (stored) {
        setConnections(JSON.parse(stored));
      }
    } catch (e) {
      console.error("Failed to load connections:", e);
    }
  };

  const saveConnections = (conns: DbConnection[]) => {
    localStorage.setItem("dbpinball_connections", JSON.stringify(conns));
    setConnections(conns);
  };

  const handleAddConnection = async () => {
    if (!formData.name || !formData.host || !formData.database || !formData.username || !password) {
      alert("Please fill in all fields");
      return;
    }

    const id = `conn_${Date.now()}`;
    const newConn: DbConnection = {
      id,
      name: formData.name,
      engine: formData.engine as "mysql" | "postgres",
      host: formData.host,
      port: formData.port || 3306,
      database: formData.database,
      username: formData.username,
      useSsl: formData.useSsl || false,
    };

    try {
      await invoke("store_password", { id, password });
      saveConnections([...connections, newConn]);
      setFormData({ engine: "mysql", port: 3306, useSsl: false });
      setPassword("");
      setOpen(false);
    } catch (e) {
      alert(`Failed to save connection: ${e}`);
    }
  };

  const handleTestConnection = async (conn: DbConnection) => {
    setTesting(conn.id);
    try {
      await invoke("test_conn", { conn });
      setTestResult((prev) => ({ ...prev, [conn.id]: true }));
    } catch (e) {
      setTestResult((prev) => ({ ...prev, [conn.id]: false }));
      alert(`Connection failed: ${e}`);
    } finally {
      setTesting(null);
    }
  };

  const handleDeleteConnection = (id: string) => {
    if (confirm("Are you sure you want to delete this connection?")) {
      invoke("delete_password", { id }).catch(console.error);
      saveConnections(connections.filter((c) => c.id !== id));
    }
  };

  return (
    <div className="space-y-6">
      <div className="flex justify-between items-center">
        <div>
          <h2 className="text-2xl font-bold">Database Connections</h2>
          <p className="text-sm text-muted-foreground">Manage your database connections</p>
        </div>
        <Dialog open={open} onOpenChange={setOpen}>
          <DialogTrigger asChild>
            <Button>
              <Plus className="w-4 h-4 mr-2" />
              New Connection
            </Button>
          </DialogTrigger>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Add Database Connection</DialogTitle>
              <DialogDescription>Create a new database connection</DialogDescription>
            </DialogHeader>
            <div className="space-y-4">
              <div>
                <Label htmlFor="name">Connection Name</Label>
                <Input
                  id="name"
                  placeholder="My Database"
                  value={formData.name || ""}
                  onChange={(e) => setFormData({ ...formData, name: e.target.value })}
                />
              </div>

              <div>
                <Label htmlFor="engine">Database Engine</Label>
                <Select value={formData.engine} onValueChange={(v) => setFormData({ ...formData, engine: v as "mysql" | "postgres" })}>
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="mysql">MySQL</SelectItem>
                    <SelectItem value="postgres">PostgreSQL</SelectItem>
                  </SelectContent>
                </Select>
              </div>

              <div className="grid grid-cols-2 gap-4">
                <div>
                  <Label htmlFor="host">Host</Label>
                  <Input
                    id="host"
                    placeholder="localhost"
                    value={formData.host || ""}
                    onChange={(e) => setFormData({ ...formData, host: e.target.value })}
                  />
                </div>
                <div>
                  <Label htmlFor="port">Port</Label>
                  <Input
                    id="port"
                    type="number"
                    value={formData.port || ""}
                    onChange={(e) => setFormData({ ...formData, port: parseInt(e.target.value) })}
                  />
                </div>
              </div>

              <div>
                <Label htmlFor="database">Database</Label>
                <Input
                  id="database"
                  placeholder="mydb"
                  value={formData.database || ""}
                  onChange={(e) => setFormData({ ...formData, database: e.target.value })}
                />
              </div>

              <div>
                <Label htmlFor="username">Username</Label>
                <Input
                  id="username"
                  placeholder="root"
                  value={formData.username || ""}
                  onChange={(e) => setFormData({ ...formData, username: e.target.value })}
                />
              </div>

              <div>
                <Label htmlFor="password">Password</Label>
                <Input
                  id="password"
                  type="password"
                  placeholder="••••••••"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                />
              </div>

              <div className="flex gap-2">
                <Button onClick={handleAddConnection} className="flex-1">
                  Save Connection
                </Button>
                <Button variant="outline" onClick={() => setOpen(false)} className="flex-1">
                  Cancel
                </Button>
              </div>
            </div>
          </DialogContent>
        </Dialog>
      </div>

      <div className="grid gap-4">
        {connections.length === 0 ? (
          <Card>
            <CardContent className="pt-6">
              <p className="text-center text-muted-foreground">No connections yet. Create one to get started.</p>
            </CardContent>
          </Card>
        ) : (
          connections.map((conn) => (
            <Card key={conn.id}>
              <CardHeader>
                <div className="flex justify-between items-start">
                  <div>
                    <CardTitle>{conn.name}</CardTitle>
                    <CardDescription>
                      {conn.engine} • {conn.username}@{conn.host}:{conn.port}/{conn.database}
                    </CardDescription>
                  </div>
                  <div className="flex gap-2">
                    {testResult[conn.id] !== undefined && (
                      <div className="flex items-center gap-1">
                        {testResult[conn.id] ? (
                          <CheckCircle className="w-5 h-5 text-green-600" />
                        ) : (
                          <AlertCircle className="w-5 h-5 text-red-600" />
                        )}
                      </div>
                    )}
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => handleTestConnection(conn)}
                      disabled={testing === conn.id}
                    >
                      {testing === conn.id ? "Testing..." : "Test"}
                    </Button>
                    <Button
                      variant="destructive"
                      size="sm"
                      onClick={() => handleDeleteConnection(conn.id)}
                    >
                      <Trash2 className="w-4 h-4" />
                    </Button>
                  </div>
                </div>
              </CardHeader>
            </Card>
          ))
        )}
      </div>
    </div>
  );
}
