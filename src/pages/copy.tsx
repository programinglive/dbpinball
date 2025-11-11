import { useState, useEffect } from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Progress } from "@/components/ui/progress";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Copy as CopyIcon, ArrowRight } from "lucide-react";

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

interface CopyOptions {
  structure: boolean;
  data: boolean;
  dropInTarget: boolean;
}

export default function CopyPage() {
  const [connections, setConnections] = useState<DbConnection[]>([]);
  const [srcConn, setSrcConn] = useState<string>("");
  const [dstConn, setDstConn] = useState<string>("");
  const [running, setRunning] = useState(false);
  const [logs, setLogs] = useState<string[]>([]);
  const [result, setResult] = useState<string | null>(null);
  const [options, setOptions] = useState<CopyOptions>({
    structure: true,
    data: true,
    dropInTarget: false,
  });

  useEffect(() => {
    loadConnections();
    const unlisten = listen<string>("job-progress", (e) => {
      setLogs((prev) => [...prev, e.payload]);
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  const loadConnections = () => {
    try {
      const stored = localStorage.getItem("dbpinball_connections");
      if (stored) {
        setConnections(JSON.parse(stored));
      }
    } catch (e) {
      console.error("Failed to load connections:", e);
    }
  };

  const handleStartCopy = async () => {
    if (!srcConn || !dstConn) {
      alert("Please select source and target connections");
      return;
    }

    if (srcConn === dstConn) {
      alert("Source and target must be different");
      return;
    }

    const src = connections.find((c) => c.id === srcConn);
    const dst = connections.find((c) => c.id === dstConn);
    if (!src || !dst) return;

    setRunning(true);
    setLogs([]);
    setResult(null);

    try {
      const file: string = await invoke("copy_db", {
        src,
        dst,
        opts: options,
      });
      setResult(file);
      setLogs((prev) => [...prev, `✓ Copy completed successfully`]);
    } catch (e) {
      setLogs((prev) => [...prev, `✗ Error: ${e}`]);
    } finally {
      setRunning(false);
    }
  };

  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-2xl font-bold">Copy Database</h2>
        <p className="text-sm text-muted-foreground">Copy a database from source to target</p>
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        <div className="lg:col-span-1 space-y-4">
          <Card>
            <CardHeader>
              <CardTitle>Copy Settings</CardTitle>
            </CardHeader>
            <CardContent className="space-y-4">
              <div>
                <Label htmlFor="source">Source Database</Label>
                <Select value={srcConn} onValueChange={setSrcConn} disabled={running}>
                  <SelectTrigger>
                    <SelectValue placeholder="Select source" />
                  </SelectTrigger>
                  <SelectContent>
                    {connections.map((conn) => (
                      <SelectItem key={conn.id} value={conn.id}>
                        {conn.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              <div className="flex justify-center py-2">
                <ArrowRight className="w-5 h-5 text-muted-foreground" />
              </div>

              <div>
                <Label htmlFor="target">Target Database</Label>
                <Select value={dstConn} onValueChange={setDstConn} disabled={running}>
                  <SelectTrigger>
                    <SelectValue placeholder="Select target" />
                  </SelectTrigger>
                  <SelectContent>
                    {connections.map((conn) => (
                      <SelectItem key={conn.id} value={conn.id}>
                        {conn.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              <div className="space-y-3">
                <Label>Content</Label>
                <div className="flex items-center space-x-2">
                  <Checkbox
                    id="structure"
                    checked={options.structure}
                    onCheckedChange={(checked) =>
                      setOptions({ ...options, structure: checked as boolean })
                    }
                    disabled={running}
                  />
                  <Label htmlFor="structure" className="font-normal cursor-pointer">
                    Structure
                  </Label>
                </div>
                <div className="flex items-center space-x-2">
                  <Checkbox
                    id="data"
                    checked={options.data}
                    onCheckedChange={(checked) =>
                      setOptions({ ...options, data: checked as boolean })
                    }
                    disabled={running}
                  />
                  <Label htmlFor="data" className="font-normal cursor-pointer">
                    Data
                  </Label>
                </div>
              </div>

              <div className="space-y-3">
                <Label>Options</Label>
                <div className="flex items-center space-x-2">
                  <Checkbox
                    id="dropInTarget"
                    checked={options.dropInTarget}
                    onCheckedChange={(checked) =>
                      setOptions({ ...options, dropInTarget: checked as boolean })
                    }
                    disabled={running}
                  />
                  <Label htmlFor="dropInTarget" className="font-normal cursor-pointer">
                    Drop if exists in target
                  </Label>
                </div>
              </div>

              <Button
                onClick={handleStartCopy}
                disabled={running || !srcConn || !dstConn}
                className="w-full"
              >
                <CopyIcon className="w-4 h-4 mr-2" />
                {running ? "Copying..." : "Start Copy"}
              </Button>
            </CardContent>
          </Card>
        </div>

        <div className="lg:col-span-2">
          <Card>
            <CardHeader>
              <CardTitle>Progress & Logs</CardTitle>
              <CardDescription>Real-time copy progress</CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              {running && <Progress value={undefined} className="w-full" />}

              <div className="h-64 rounded border p-3 overflow-auto bg-muted font-mono text-xs space-y-1">
                {logs.length === 0 ? (
                  <p className="text-muted-foreground">Waiting for copy to start...</p>
                ) : (
                  logs.map((log, i) => (
                    <div key={i} className="text-foreground">
                      {log}
                    </div>
                  ))
                )}
              </div>

              {result && (
                <div className="p-3 rounded bg-green-50 border border-green-200">
                  <p className="text-sm font-medium text-green-900">✓ Copy completed successfully</p>
                </div>
              )}
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  );
}
