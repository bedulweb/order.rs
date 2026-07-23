import { useCallback, useEffect, useMemo, useState } from "react";
import {
  BrowserRouter,
  Link,
  Navigate,
  NavLink,
  Route,
  Routes,
  useNavigate,
  useParams,
} from "react-router-dom";
import {
  ApiError,
  clearToken,
  createBatch,
  downloadBatchPdf,
  fetchBacklog,
  fetchBatch,
  fetchBatchesToday,
  regenerateBatchPdf,
  fetchCatalogProducts,
  formatIdr,
  formatWib,
  getToken,
  importCatalog,
  setToken,
  type BacklogOrder,
  type BacklogResponse,
  type BatchDetail,
  type BatchSession,
  type BatchSummary,
  type BatchesListResponse,
  type CatalogProduct,
} from "@/lib/api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardDescription,
  CardHeader,
  CardPanel,
  CardTitle,
} from "@/components/ui/card";
import {
  Dialog,
  DialogClose,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogPopup,
  DialogTitle,
} from "@/components/ui/dialog";
import { Empty } from "@/components/ui/empty";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { cn } from "@/lib/utils";

export default function App() {
  return (
    <BrowserRouter>
      <AppRoutes />
    </BrowserRouter>
  );
}

function AppRoutes() {
  const [token, setTokenState] = useState<string | null>(() => getToken());

  if (!token) {
    return (
      <LoginGate
        onLogin={(t) => {
          setToken(t);
          setTokenState(t);
        }}
      />
    );
  }

  return (
    <div className="min-h-svh bg-background text-foreground">
      <header className="border-b bg-card/60 backdrop-blur">
        <div className="mx-auto flex max-w-6xl items-center justify-between gap-4 px-4 py-3">
          <Link to="/" className="text-left no-underline">
            <div className="font-heading text-lg font-semibold tracking-tight text-foreground">
              Orders Ops
            </div>
            <div className="text-muted-foreground text-xs">
              Asia/Jakarta · pick lists · rs.obayito.com
            </div>
          </Link>
          <nav className="flex flex-wrap items-center gap-2">
            <NavBtn to="/" end>
              Home
            </NavBtn>
            <NavBtn to="/backlog">Backlog</NavBtn>
            <NavBtn to="/products">Products</NavBtn>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => {
                clearToken();
                setTokenState(null);
              }}
            >
              Log out
            </Button>
          </nav>
        </div>
      </header>

      <main className="mx-auto max-w-6xl px-4 py-6 text-left">
        <Routes>
          <Route path="/" element={<OpsHome />} />
          <Route path="/backlog" element={<BacklogPage />} />
          <Route path="/products" element={<ProductsPage />} />
          <Route path="/batches/:id" element={<BatchDetailPage />} />
          <Route path="*" element={<Navigate to="/" replace />} />
        </Routes>
      </main>
    </div>
  );
}

function NavBtn({
  to,
  end,
  children,
}: {
  to: string;
  end?: boolean;
  children: React.ReactNode;
}) {
  return (
    <NavLink to={to} end={end} className="no-underline">
      {({ isActive }) => (
        <span
          className={cn(
            "inline-flex h-8 items-center justify-center gap-1.5 rounded-lg border px-[calc(--spacing(2.5)-1px)] text-sm font-medium sm:h-7",
            isActive
              ? "border-primary bg-primary text-primary-foreground shadow-xs"
              : "border-input bg-popover text-foreground shadow-xs/5 hover:bg-accent/50",
          )}
        >
          {children}
        </span>
      )}
    </NavLink>
  );
}

function LoginGate({ onLogin }: { onLogin: (token: string) => void }) {
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    const t = value.trim();
    if (!t) {
      setError("Token required");
      return;
    }
    setLoading(true);
    setError(null);
    try {
      // Probe auth against backlog endpoint
      setToken(t);
      await fetchBacklog(1);
      onLogin(t);
    } catch (err) {
      clearToken();
      setError(
        err instanceof ApiError
          ? err.status === 401
            ? "Invalid token"
            : err.message
          : "Login failed",
      );
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="flex min-h-svh items-center justify-center bg-background px-4">
      <Card className="w-full max-w-md">
        <CardHeader>
          <CardTitle>Ops login</CardTitle>
          <CardDescription>
            Enter the API token (same as{" "}
            <code className="text-xs">API_TOKEN</code>). Stored in sessionStorage
            for this browser tab.
          </CardDescription>
        </CardHeader>
        <CardPanel>
          <form className="flex flex-col gap-3" onSubmit={submit}>
            <Input
              type="password"
              autoComplete="current-password"
              placeholder="Bearer token"
              value={value}
              onChange={(e) => setValue(e.target.value)}
            />
            {error && (
              <p className="text-destructive text-sm" role="alert">
                {error}
              </p>
            )}
            <Button type="submit" loading={loading}>
              Continue
            </Button>
          </form>
        </CardPanel>
      </Card>
    </div>
  );
}

function OpsHome() {
  const navigate = useNavigate();
  const [backlog, setBacklog] = useState<BacklogResponse | null>(null);
  const [batches, setBatches] = useState<BatchesListResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [pendingSession, setPendingSession] = useState<BatchSession | null>(
    null,
  );
  const [generating, setGenerating] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [b, list] = await Promise.all([fetchBacklog(), fetchBatchesToday()]);
      setBacklog(b);
      setBatches(list);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  async function confirmGenerate() {
    if (!pendingSession) return;
    setGenerating(true);
    setNotice(null);
    setError(null);
    try {
      const detail = await createBatch(pendingSession);
      setNotice(
        `Created ${detail.session} batch ${detail.id.slice(0, 8)}… (${detail.orderCount} orders)`,
      );
      setPendingSession(null);
      await load();
      void navigate(`/batches/${detail.id}`);
    } catch (err) {
      const raw = err instanceof Error ? err.message : "Generate failed";
      const friendly =
        /no eligible orders/i.test(raw)
          ? pendingSession === "urgent"
            ? "Tidak ada order urgent di backlog. Semua order new mungkin sudah masuk batch, atau tidak ada yang terklasifikasi urgent."
            : "Backlog kosong — tidak ada order eligible. Order state=new yang sudah masuk batch aktif tidak bisa digenerate lagi. Buka batch hari ini untuk unduh PDF, atau sync order baru dulu."
          : raw;
      setError(friendly);
      setPendingSession(null);
    } finally {
      setGenerating(false);
    }
  }

  const backlogEmpty = !loading && (backlog?.total ?? 0) === 0;
  const urgentEmpty = !loading && (backlog?.urgentCount ?? 0) === 0;

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="font-heading text-2xl font-semibold tracking-tight">
            Warehouse ops
          </h1>
          <p className="text-muted-foreground text-sm">
            Morning / afternoon sessions · urgent anytime · membership = source of
            truth
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={() => void load()}>
          Refresh
        </Button>
      </div>

      {notice && (
        <div className="rounded-lg border border-success/30 bg-success/8 px-3 py-2 text-sm text-success-foreground">
          {notice}
        </div>
      )}
      {error && (
        <div className="rounded-lg border border-destructive/30 bg-destructive/8 px-3 py-2 text-sm text-destructive-foreground">
          {error}
        </div>
      )}

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        <StatCard
          title="Backlog"
          value={loading ? null : String(backlog?.total ?? 0)}
          hint="state=new, not in active batch"
          action={
            <Button
              size="sm"
              variant="outline"
              render={<Link to="/backlog" />}
            >
              View table
            </Button>
          }
        />
        <StatCard
          title="Urgent in backlog"
          value={loading ? null : String(backlog?.urgentCount ?? 0)}
          hint="instant / sameday / gojek / grab / …"
        />
        <StatCard
          title="Today’s batches"
          value={loading ? null : String(batches?.batches.length ?? 0)}
          hint={batches ? `WIB day ${batches.date}` : "Asia/Jakarta"}
        />
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Generate batch</CardTitle>
          <CardDescription>
            Creates a Summary List PDF and locks orders into membership (no
            double-assign). Reprint from Today’s batches — do not re-generate
            when backlog is 0.
          </CardDescription>
        </CardHeader>
        <CardPanel className="flex flex-col gap-3">
          {backlogEmpty && (
            <p className="text-muted-foreground text-sm">
              Backlog kosong (0 eligible). Order new hari ini sudah terkunci di
              batch aktif — buka baris di “Today’s batches” untuk PDF, atau sync
              order baru.
            </p>
          )}
          <div className="flex flex-wrap gap-2">
            <Button
              disabled={backlogEmpty}
              onClick={() => setPendingSession("morning")}
            >
              Morning
            </Button>
            <Button
              variant="secondary"
              disabled={backlogEmpty}
              onClick={() => setPendingSession("afternoon")}
            >
              Afternoon
            </Button>
            <Button
              variant="outline"
              disabled={urgentEmpty}
              onClick={() => setPendingSession("urgent")}
            >
              Urgent only
            </Button>
          </div>
        </CardPanel>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Today’s batches</CardTitle>
          <CardDescription>
            Reprint PDF never re-selects backlog — same batch id.
          </CardDescription>
        </CardHeader>
        <CardPanel>
          {loading ? (
            <div className="flex flex-col gap-2">
              <Skeleton className="h-8 w-full" />
              <Skeleton className="h-8 w-full" />
            </div>
          ) : !batches?.batches.length ? (
            <Empty className="py-8">
              <p className="text-muted-foreground text-sm">No batches yet today.</p>
            </Empty>
          ) : (
            <BatchesTable rows={batches.batches} />
          )}
        </CardPanel>
      </Card>

      <Dialog
        open={pendingSession !== null}
        onOpenChange={(open) => {
          if (!open && !generating) setPendingSession(null);
        }}
      >
        <DialogPopup>
          <DialogHeader>
            <DialogTitle>Generate {pendingSession} batch?</DialogTitle>
            <DialogDescription>
              {pendingSession === "urgent"
                ? "Only urgent-classified backlog orders will be included."
                : "All eligible backlog orders will be included (urgent first)."}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <DialogClose
              render={<Button variant="outline" disabled={generating} />}
            >
              Cancel
            </DialogClose>
            <Button loading={generating} onClick={() => void confirmGenerate()}>
              Generate
            </Button>
          </DialogFooter>
        </DialogPopup>
      </Dialog>
    </div>
  );
}

function StatCard({
  title,
  value,
  hint,
  action,
}: {
  title: string;
  value: string | null;
  hint: string;
  action?: React.ReactNode;
}) {
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardDescription>{title}</CardDescription>
        <CardTitle className="text-3xl tabular-nums">
          {value === null ? <Skeleton className="h-9 w-16" /> : value}
        </CardTitle>
      </CardHeader>
      <CardPanel className="flex items-center justify-between gap-2">
        <p className="text-muted-foreground text-xs">{hint}</p>
        {action}
      </CardPanel>
    </Card>
  );
}

function BatchesTable({ rows }: { rows: BatchSummary[] }) {
  const [busy, setBusy] = useState<string | null>(null);

  return (
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead>Session</TableHead>
          <TableHead>Created (WIB)</TableHead>
          <TableHead>Orders</TableHead>
          <TableHead>Urgent</TableHead>
          <TableHead className="text-right">Actions</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {rows.map((b) => (
          <TableRow key={b.id}>
            <TableCell className="font-medium capitalize">{b.session}</TableCell>
            <TableCell className="text-muted-foreground text-xs">
              {b.createdAtWib}
            </TableCell>
            <TableCell className="tabular-nums">{b.orderCount}</TableCell>
            <TableCell className="tabular-nums">{b.urgentCount}</TableCell>
            <TableCell className="text-right">
              <div className="flex justify-end gap-1">
                <Button
                  size="xs"
                  variant="outline"
                  render={<Link to={`/batches/${b.id}`} />}
                >
                  Detail
                </Button>
                <Button
                  size="xs"
                  variant="secondary"
                  loading={busy === b.id}
                  onClick={() => {
                    setBusy(b.id);
                    void downloadBatchPdf(b.id, b.pdfFilename ?? undefined)
                      .catch((e: unknown) =>
                        alert(e instanceof Error ? e.message : "PDF failed"),
                      )
                      .finally(() => setBusy(null));
                  }}
                >
                  PDF
                </Button>
              </div>
            </TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  );
}

function BacklogPage() {
  const [data, setData] = useState<BacklogResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    void (async () => {
      setLoading(true);
      try {
        setData(await fetchBacklog(2000));
      } catch (err) {
        setError(err instanceof Error ? err.message : "Failed");
      } finally {
        setLoading(false);
      }
    })();
  }, []);

  const rows: BacklogOrder[] = data?.orders ?? [];

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center justify-between gap-2">
        <div>
          <Button size="sm" variant="ghost" render={<Link to="/" />}>
            ← Home
          </Button>
          <h1 className="font-heading text-2xl font-semibold">Backlog</h1>
          <p className="text-muted-foreground text-sm">
            {data
              ? `${data.total} orders · ${data.urgentCount} urgent`
              : "Loading…"}
          </p>
        </div>
      </div>
      {error && (
        <p className="text-destructive text-sm" role="alert">
          {error}
        </p>
      )}
      {loading ? (
        <Skeleton className="h-40 w-full" />
      ) : rows.length === 0 ? (
        <Empty className="py-12">
          <p className="text-muted-foreground text-sm">Backlog is empty.</p>
        </Empty>
      ) : (
        <Card>
          <CardPanel className="pt-4">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Platform ID</TableHead>
                  <TableHead>Platform</TableHead>
                  <TableHead>Carrier</TableHead>
                  <TableHead>Ordered (WIB)</TableHead>
                  <TableHead>Flag</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {rows.map((o) => (
                  <TableRow key={o.orderId}>
                    <TableCell className="font-mono text-xs">
                      {o.platformOrderId}
                    </TableCell>
                    <TableCell>{o.platform}</TableCell>
                    <TableCell className="max-w-[14rem] truncate text-sm">
                      {o.carrier ?? "—"}
                    </TableCell>
                    <TableCell className="text-muted-foreground text-xs">
                      {formatWib(o.orderedAt)}
                    </TableCell>
                    <TableCell>
                      {o.isUrgent ? (
                        <Badge variant="warning">Urgent</Badge>
                      ) : (
                        <Badge variant="outline">Normal</Badge>
                      )}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </CardPanel>
        </Card>
      )}
    </div>
  );
}

function ProductsPage() {
  const [rows, setRows] = useState<CatalogProduct[]>([]);
  const [total, setTotal] = useState(0);
  const [q, setQ] = useState("");
  const [search, setSearch] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [importing, setImporting] = useState(false);

  const load = useCallback(async (query: string) => {
    setLoading(true);
    setError(null);
    try {
      const resp = await fetchCatalogProducts({
        q: query || undefined,
        limit: 500,
      });
      setRows(resp.products);
      setTotal(resp.total);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load catalog");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load(search);
  }, [load, search]);

  async function runImport() {
    setImporting(true);
    setNotice(null);
    setError(null);
    try {
      const r = await importCatalog();
      setNotice(
        `Import done: inserted ${r.inserted}, updated ${r.updated}, skipped ${r.skipped} (rows ${r.totalRows})`,
      );
      await load(search);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Import failed");
    } finally {
      setImporting(false);
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <Button size="sm" variant="ghost" render={<Link to="/" />}>
            ← Home
          </Button>
          <h1 className="font-heading text-2xl font-semibold">Products</h1>
          <p className="text-muted-foreground text-sm">
            Catalog by ART (SKU) · HPP in IDR · {total} products
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <form
            className="flex gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              setSearch(q.trim());
            }}
          >
            <Input
              placeholder="Search ART or name"
              value={q}
              onChange={(e) => setQ(e.target.value)}
              className="w-48 sm:w-64"
            />
            <Button type="submit" size="sm" variant="outline">
              Search
            </Button>
          </form>
          <Button size="sm" loading={importing} onClick={() => void runImport()}>
            Import workbook
          </Button>
        </div>
      </div>

      {notice && (
        <div className="rounded-lg border border-success/30 bg-success/8 px-3 py-2 text-sm text-success-foreground">
          {notice}
        </div>
      )}
      {error && (
        <p className="text-destructive text-sm" role="alert">
          {error}
        </p>
      )}

      {loading ? (
        <Skeleton className="h-40 w-full" />
      ) : rows.length === 0 ? (
        <Empty className="py-12">
          <p className="text-muted-foreground text-sm">
            No products yet. Run Import workbook (server path
            MARKETPLACE_PRICE_2026_NORMALIZED.xlsx).
          </p>
        </Empty>
      ) : (
        <Card>
          <CardPanel className="pt-4">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>ART</TableHead>
                  <TableHead>Name</TableHead>
                  <TableHead className="text-right">HPP</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {rows.map((p) => (
                  <TableRow key={p.art}>
                    <TableCell className="font-mono text-xs">{p.art}</TableCell>
                    <TableCell className="max-w-[28rem] truncate text-sm">
                      {p.name || "—"}
                    </TableCell>
                    <TableCell className="text-right tabular-nums text-sm">
                      {formatIdr(p.hpp)}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </CardPanel>
        </Card>
      )}
    </div>
  );
}

function BatchDetailPage() {
  const { id: idParam } = useParams<{ id: string }>();
  const id = idParam ?? "";
  const [detail, setDetail] = useState<BatchDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [pdfBusy, setPdfBusy] = useState(false);
  const [regenBusy, setRegenBusy] = useState(false);

  useEffect(() => {
    if (!id) return;
    void (async () => {
      setLoading(true);
      try {
        setDetail(await fetchBatch(id));
      } catch (err) {
        setError(err instanceof Error ? err.message : "Failed");
      } finally {
        setLoading(false);
      }
    })();
  }, [id]);

  const members = useMemo(() => detail?.members ?? [], [detail]);

  if (!id) {
    return <Navigate to="/" replace />;
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <Button size="sm" variant="ghost" render={<Link to="/" />}>
            ← Home
          </Button>
          <h1 className="font-heading text-2xl font-semibold">Batch detail</h1>
          {detail && (
            <p className="text-muted-foreground text-sm">
              <span className="capitalize">{detail.session}</span> ·{" "}
              {detail.createdAtWib} · {detail.orderCount} orders (
              {detail.urgentCount} urgent)
            </p>
          )}
          <p className="mt-1 font-mono text-xs text-muted-foreground">{id}</p>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button
            variant="outline"
            loading={regenBusy}
            disabled={!detail || pdfBusy}
            onClick={() => {
              setRegenBusy(true);
              setError(null);
              void regenerateBatchPdf(id)
                .then((d) => {
                  setDetail(d);
                  setPdfBusy(true);
                  return downloadBatchPdf(id, d.pdfFilename ?? undefined);
                })
                .catch((e: unknown) =>
                  setError(e instanceof Error ? e.message : "Regenerate failed"),
                )
                .finally(() => {
                  setRegenBusy(false);
                  setPdfBusy(false);
                });
            }}
          >
            Rebuild PDF
          </Button>
          <Button
            loading={pdfBusy}
            disabled={!detail || regenBusy}
            onClick={() => {
              setPdfBusy(true);
              void downloadBatchPdf(id, detail?.pdfFilename ?? undefined)
                .catch((e: unknown) =>
                  alert(e instanceof Error ? e.message : "PDF failed"),
                )
                .finally(() => setPdfBusy(false));
            }}
          >
            Download PDF
          </Button>
        </div>
      </div>

      {error && (
        <p className="text-destructive text-sm" role="alert">
          {error}
        </p>
      )}
      {loading ? (
        <Skeleton className="h-40 w-full" />
      ) : (
        <Card>
          <CardPanel className="pt-4">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>#</TableHead>
                  <TableHead>Platform ID</TableHead>
                  <TableHead>Carrier</TableHead>
                  <TableHead>Items</TableHead>
                  <TableHead>Flag</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {members.map((m) => (
                  <TableRow key={m.orderId}>
                    <TableCell className="tabular-nums text-muted-foreground">
                      {m.position + 1}
                    </TableCell>
                    <TableCell className="font-mono text-xs">
                      {m.platformOrderId}
                    </TableCell>
                    <TableCell className="max-w-[12rem] truncate text-sm">
                      {m.carrierSnapshot ?? "—"}
                    </TableCell>
                    <TableCell className="text-xs">
                      {m.items.length === 0
                        ? "—"
                        : m.items
                            .map(
                              (it) =>
                                `x${it.quantity} ${it.sku ?? it.name ?? ""}`,
                            )
                            .join(", ")}
                    </TableCell>
                    <TableCell>
                      {m.isUrgent ? (
                        <Badge variant="warning">Urgent</Badge>
                      ) : (
                        <Badge variant="outline">Normal</Badge>
                      )}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </CardPanel>
        </Card>
      )}
    </div>
  );
}
