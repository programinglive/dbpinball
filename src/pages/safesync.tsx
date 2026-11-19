import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ShieldCheck, Play, SearchCheck, FileCheck2, AlertTriangle, DatabaseBackup } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import type { SshTunnelConfig, TargetClass } from "@/pages/connections.types";

export interface SafeSyncConnection {
  id: string;
  name: string;
  engine: "mysql" | "postgres";
  host: string;
  port: number;
  database: string;
  username: string;
  useSsl: boolean;
  target?: TargetClass;
  sshTunnel?: SshTunnelConfig;
}

const LOCAL_DESTINATION_HOSTS = new Set(["127.0.0.1", "localhost", "::1"]);

/**
 * Only local, non-production, non-SSH-tunneled connections may be selected as
 * a Safe Sync destination.
 */
export function isLocalDestinationConnection(conn: SafeSyncConnection): boolean {
  if (conn.target === "production") return false;
  if (conn.sshTunnel?.enabled) return false;
  return LOCAL_DESTINATION_HOSTS.has(conn.host.trim().toLowerCase());
}

interface SecurityStatus {
  enrolled: boolean;
  category?: string;
  fingerprintMatch?: boolean;
  [key: string]: unknown;
}

interface SessionIdentity {
  database?: string;
  user?: string;
  server?: {
    engine?: string;
    version?: string;
    [key: string]: unknown;
  };
  [key: string]: unknown;
}

interface DryRunPlan {
  plan_fingerprint?: string;
  planId?: string;
  engine?: string;
  expires_at_ms?: number;
  tables?: Array<{ table?: string; estimated_rows?: number; [key: string]: unknown }>;
  [key: string]: unknown;
}

interface ValidationReport {
  valid?: boolean;
  warnings?: string[];
  [key: string]: unknown;
}

interface RunReport {
  run_id?: string;
  runId?: string;
  status?: string;
  rowsWritten?: number;
  rowsImported?: number;
  [key: string]: unknown;
}

interface FullCopyReport {
  engine: "mysql" | "postgres";
  sourceDatabase: string;
  destinationDatabase: string;
  tablesCopied: number;
  rowsCopied: number;
  replaced: boolean;
  durationMs: number;
  tables: Array<{ table: string; rowsCopied: number }>;
}

const DEFAULT_SPEC = `version: 1
source: source
destination: destination
selection:
  roots:
    - table: orders
      filter:
        - column: id
          op: gte
          value: 1
masking:
  policy: development-default@2
`;

export default function SafeSyncPage() {
  const [connections, setConnections] = useState<SafeSyncConnection[]>([]);
  const [sourceId, setSourceId] = useState<string>("");
  const [destinationId, setDestinationId] = useState<string>("");
  const [sourceIdentity, setSourceIdentity] = useState<SessionIdentity | null>(null);
  const [destIdentity, setDestIdentity] = useState<SessionIdentity | null>(null);
  const [sourceSecurity, setSourceSecurity] = useState<SecurityStatus | null>(null);
  const [destSecurity, setDestSecurity] = useState<SecurityStatus | null>(null);
  const [specText, setSpecText] = useState<string>(DEFAULT_SPEC);
  const [estimate, setEstimate] = useState<boolean>(false);
  const [replace, setReplace] = useState<boolean>(false);
  const [plan, setPlan] = useState<DryRunPlan | null>(null);
  const [validation, setValidation] = useState<ValidationReport | null>(null);
  const [acknowledgement, setAcknowledgement] = useState<string>("");
  const [runReport, setRunReport] = useState<RunReport | null>(null);
  const [runStatus, setRunStatus] = useState<Record<string, unknown> | null>(null);
  const [syncMode, setSyncMode] = useState<"subset" | "full">("subset");
  const [fullReplace, setFullReplace] = useState<boolean>(true);
  const [fullConfirmDb, setFullConfirmDb] = useState<string>("");
  const [fullCopyReport, setFullCopyReport] = useState<FullCopyReport | null>(null);
  const [busy, setBusy] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<SafeSyncConnection[]>("connections_list")
      .then((list) => setConnections(list))
      .catch((err) => setError(String(err)));
  }, []);

  const destinationConnections = useMemo(
    () => connections.filter(isLocalDestinationConnection),
    [connections],
  );
  const sourceConn = connections.find((c) => c.id === sourceId) ?? null;
  const destConn = destinationConnections.find((c) => c.id === destinationId) ?? null;

  const handleInspect = async () => {
    if (!sourceConn || !destConn) {
      setError("Select both source and destination connections first.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const [srcId, dstId, srcSec, dstSec] = await Promise.all([
        invoke<SessionIdentity>("safe_sync_inspect", { connection: sourceConn }),
        invoke<SessionIdentity>("safe_sync_inspect", { connection: destConn }),
        invoke<SecurityStatus>("security_status", { connection: sourceConn }),
        invoke<SecurityStatus>("security_status", { connection: destConn }),
      ]);
      setSourceIdentity(srcId);
      setDestIdentity(dstId);
      setSourceSecurity(srcSec);
      setDestSecurity(dstSec);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const handlePlan = async () => {
    if (!sourceConn || !destConn) {
      setError("Select both source and destination connections first.");
      return;
    }
    setBusy(true);
    setError(null);
    setValidation(null);
    setRunReport(null);
    try {
      const result = await invoke<DryRunPlan>("safe_sync_plan", {
        specYamlOrJson: specText,
        source: sourceConn,
        destination: destConn,
        estimate,
        replace,
      });
      setPlan(result);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const handleValidate = async () => {
    if (!plan || !sourceConn || !destConn) return;
    setBusy(true);
    setError(null);
    try {
      const result = await invoke<ValidationReport>("safe_sync_validate", {
        plan,
        specYamlOrJson: specText,
        source: sourceConn,
        destination: destConn,
      });
      setValidation(result);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const handleExecute = async () => {
    if (!plan || !sourceConn || !destConn) return;
    if (!acknowledgement.trim()) {
      setError("Explicit acknowledgement is required before mutating the destination database.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const report = await invoke<RunReport>("safe_sync_execute", {
        plan,
        specYamlOrJson: specText,
        acknowledgement: acknowledgement.trim(),
        replace,
        source: sourceConn,
        destination: destConn,
      });
      setRunReport(report);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const handleCheckStatus = async () => {
    const runId = runReport?.run_id ?? runReport?.runId;
    if (!destConn || !runId) return;
    setBusy(true);
    setError(null);
    try {
      const status = await invoke<Record<string, unknown>>("safe_sync_status", {
        runId,
        destination: destConn,
      });
      setRunStatus(status);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const handleFullCopy = async () => {
    if (!sourceConn || !destConn) {
      setError("Select both source and destination connections first.");
      return;
    }
    if (fullConfirmDb.trim() !== destConn.database) {
      setError(`Type the destination database name (${destConn.database}) to confirm Full Copy.`);
      return;
    }
    setBusy(true);
    setError(null);
    setFullCopyReport(null);
    try {
      const report = await invoke<FullCopyReport>("safe_sync_full_copy", {
        source: sourceConn,
        destination: destConn,
        replace: fullReplace,
      });
      setFullCopyReport(report);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="mx-auto max-w-4xl space-y-5 p-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 className="text-2xl font-bold flex items-center gap-2">
            <ShieldCheck className="h-6 w-6" />
            Safe Sync
          </h2>
          <p className="text-sm text-muted-foreground">
            Bounded, masked subset sync or full local schema + data copy for PostgreSQL and MariaDB.
          </p>
        </div>
        <div className="inline-flex rounded-lg border border-border bg-muted/40 p-1 gap-1" role="tablist" aria-label="Sync mode">
          <Button
            type="button"
            size="sm"
            variant={syncMode === "subset" ? "default" : "ghost"}
            onClick={() => setSyncMode("subset")}
          >
            Subset Sync (Masked)
          </Button>
          <Button
            type="button"
            size="sm"
            variant={syncMode === "full" ? "default" : "ghost"}
            onClick={() => setSyncMode("full")}
          >
            <DatabaseBackup className="mr-1.5 h-3.5 w-3.5" />
            Full Copy
          </Button>
        </div>
      </div>

      {error && (
        <div role="alert" className="rounded-md border border-destructive/50 bg-destructive/10 p-3 text-sm text-destructive">
          {error}
        </div>
      )}

      <Card>
        <CardHeader>
          <CardTitle>1. Source &amp; Destination Connections</CardTitle>
          <CardDescription>Inspect server identities and enrolled security posture before planning.</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="grid gap-4 md:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="safesync-source">Source (Read-Only)</Label>
              <Select value={sourceId} onValueChange={setSourceId}>
                <SelectTrigger id="safesync-source" aria-label="Source connection">
                  <SelectValue placeholder="Select source connection" />
                </SelectTrigger>
                <SelectContent>
                  {connections.map((conn) => (
                    <SelectItem key={conn.id} value={conn.id}>
                      {conn.name} ({conn.engine} · {conn.database})
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-2">
              <Label htmlFor="safesync-dest">Destination (Local/Dev Only)</Label>
              <Select value={destinationId} onValueChange={setDestinationId} disabled={destinationConnections.length === 0}>
                <SelectTrigger id="safesync-dest" aria-label="Destination connection">
                  <SelectValue
                    placeholder={
                      destinationConnections.length === 0
                        ? "No localhost destination available"
                        : "Select destination connection"
                    }
                  />
                </SelectTrigger>
                <SelectContent>
                  {destinationConnections.map((conn) => (
                    <SelectItem key={conn.id} value={conn.id}>
                      {conn.name} ({conn.engine} · {conn.database})
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {destinationConnections.length === 0 && (
                <p className="text-xs text-muted-foreground">
                  Destination must be a local database (127.0.0.1 / localhost, non-production, without SSH tunnel). Add one in Connections first.
                </p>
              )}
            </div>
          </div>

          <Button type="button" variant="outline" disabled={busy || !sourceConn || !destConn} onClick={handleInspect}>
            <SearchCheck className="mr-2 h-4 w-4" />
            Inspect Identities &amp; Security
          </Button>

          {(sourceIdentity || destIdentity) && (
            <div className="grid gap-4 md:grid-cols-2 text-xs">
              <pre className="rounded bg-muted p-3 overflow-auto" data-testid="source-identity">
                {JSON.stringify({ identity: sourceIdentity, security: sourceSecurity }, null, 2)}
              </pre>
              <pre className="rounded bg-muted p-3 overflow-auto" data-testid="destination-identity">
                {JSON.stringify({ identity: destIdentity, security: destSecurity }, null, 2)}
              </pre>
            </div>
          )}
        </CardContent>
      </Card>

      {syncMode === "full" ? (
        <Card>
          <CardHeader>
            <CardTitle>2. Full Database Copy (Schema + Data)</CardTitle>
            <CardDescription>
              Copies all base tables (schema and rows) from Source to your enrolled local destination without needing a YAML spec.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="flex items-start gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 p-3 text-xs">
              <AlertTriangle className="h-4 w-4 shrink-0 text-amber-600" />
              <span>
                Full Copy writes unmasked production data to your local/dev destination. Only enrolled localhost non-production databases are allowed.
              </span>
            </div>

            <label className="flex items-center gap-2 text-sm">
              <Checkbox checked={fullReplace} onCheckedChange={(v) => setFullReplace(Boolean(v))} />
              <span>Replace existing tables in destination (DROP &amp; recreate)</span>
            </label>

            <div className="space-y-2">
              <Label htmlFor="fullcopy-confirm">Confirm Destination Database Name</Label>
              <Input
                id="fullcopy-confirm"
                placeholder={destConn ? `Type ${destConn.database}` : "Select a destination connection first"}
                value={fullConfirmDb}
                onChange={(e) => setFullConfirmDb(e.target.value)}
              />
            </div>

            <Button
              type="button"
              disabled={busy || !sourceConn || !destConn || fullConfirmDb.trim() !== destConn?.database}
              onClick={handleFullCopy}
            >
              <DatabaseBackup className="mr-2 h-4 w-4" />
              {busy ? "Copying Database..." : "Run Full Copy"}
            </Button>

            {fullCopyReport && (
              <pre className="rounded bg-muted p-3 text-xs overflow-auto" data-testid="full-copy-report">
                {JSON.stringify(fullCopyReport, null, 2)}
              </pre>
            )}
          </CardContent>
        </Card>
      ) : (
        <>
          <Card>
            <CardHeader>
              <CardTitle>2. Safe Sync Specification &amp; Dry-Run Plan</CardTitle>
          <CardDescription>Define subset root, row bounds, and masking policy in YAML or JSON.</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <Textarea
            aria-label="Safe Sync specification"
            rows={8}
            value={specText}
            onChange={(e) => setSpecText(e.target.value)}
          />
          <div className="flex flex-wrap items-center gap-6">
            <label className="flex items-center gap-2 text-sm">
              <Checkbox checked={estimate} onCheckedChange={(v) => setEstimate(Boolean(v))} />
              <span>Estimate row counts</span>
            </label>
            <label className="flex items-center gap-2 text-sm">
              <Checkbox checked={replace} onCheckedChange={(v) => setReplace(Boolean(v))} />
              <span>Replace existing destination rows</span>
            </label>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button type="button" disabled={busy || !sourceConn || !destConn} onClick={handlePlan}>
              <FileCheck2 className="mr-2 h-4 w-4" />
              Generate Dry-Run Plan
            </Button>
            <Button type="button" variant="secondary" disabled={busy || !plan} onClick={handleValidate}>
              Validate Plan
            </Button>
          </div>
          {plan && (
            <pre className="rounded bg-muted p-3 text-xs overflow-auto" data-testid="plan-preview">
              {JSON.stringify(plan, null, 2)}
            </pre>
          )}
          {validation && (
            <pre className="rounded bg-muted p-3 text-xs overflow-auto" data-testid="validation-preview">
              {JSON.stringify(validation, null, 2)}
            </pre>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>3. Execute with Explicit Acknowledgement</CardTitle>
          <CardDescription>
            Destination mutations run inside a single transaction. Confirm the destination identity before executing.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex items-start gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 p-3 text-xs">
            <AlertTriangle className="h-4 w-4 shrink-0 text-amber-600" />
            <span>
              {replace
                ? "Replace mode will delete existing rows in target tables in reverse topological order before inserting masked rows."
                : "Append/upsert mode will write masked subset rows into the enrolled local/dev destination."}
            </span>
          </div>
          <div className="space-y-2">
            <Label htmlFor="safesync-ack">Acknowledgement Phrase (plan_fingerprint)</Label>
            <Input
              id="safesync-ack"
              placeholder={
                plan?.plan_fingerprint
                  ? `Enter ${plan.plan_fingerprint}`
                  : "Enter exact plan_fingerprint from Dry-Run Plan"
              }
              value={acknowledgement}
              onChange={(e) => setAcknowledgement(e.target.value)}
            />
          </div>
          <div className="flex flex-wrap gap-2">
            <Button type="button" disabled={busy || !plan || !acknowledgement.trim()} onClick={handleExecute}>
              <Play className="mr-2 h-4 w-4" />
              Execute Safe Sync
            </Button>
            {(runReport?.run_id ?? runReport?.runId) && (
              <Button type="button" variant="outline" disabled={busy} onClick={handleCheckStatus}>
                Refresh Run Status
              </Button>
            )}
          </div>
          {runReport && (
            <pre className="rounded bg-muted p-3 text-xs overflow-auto" data-testid="run-report">
              {JSON.stringify(runReport, null, 2)}
            </pre>
          )}
          {runStatus && (
            <pre className="rounded bg-muted p-3 text-xs overflow-auto" data-testid="run-status">
              {JSON.stringify(runStatus, null, 2)}
            </pre>
          )}
        </CardContent>
      </Card>
        </>
      )}
    </div>
  );
}
