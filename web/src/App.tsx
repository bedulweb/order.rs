import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  BrowserRouter,
  Link,
  Navigate,
  NavLink,
  Route,
  Routes,
  useLocation,
  useNavigate,
  useParams,
} from "react-router-dom";
import {
  flexRender,
  getCoreRowModel,
  useReactTable,
  type ColumnDef,
} from "@tanstack/react-table";
import {
  BadgeCheck,
  ChevronLeft,
  ChevronRight,
  Circle,
  CircleCheck,
  CircleX,
  LoaderCircle,
  PackageCheck,
  Printer,
  Search,
  X,
  Zap,
} from "lucide-react";
import {
  ApiError,
  clearToken,
  createBatch,
  fetchAnalytics,
  createBatchFromSelection,
  downloadBatchPdf,
  downloadPicklistPdf,
  fetchBacklog,
  fetchBatch,
  fetchBatchesToday,
  regenerateBatchPdf,
  fetchCatalogProducts,
  fetchOrdersFeed,
  fetchSyncProgress,
  fetchTodayStats,
  formatIdr,
  formatWib,
  getToken,
  importCatalog,
  confirmResiPrint,
  packOrders,
  prepareResiPrint,
  setToken,
  startSyncRun,
  type Analytics,
  type AnalyticsProduct,
  type BacklogOrder,
  type BacklogResponse,
  type BatchDetail,
  type BatchSession,
  type BatchSummary,
  type BatchesListResponse,
  type CatalogProduct,
  type FeedStatus,
  type NewOrder,
  type NewOrderItem,
  type OrdersFeedResponse,
  type PackResult,
  type ResiPrep,
  type SelectionBatchResult,
  type SyncProgress,
  type SyncStepState,
  type TodayStats,
} from "@/lib/api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
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
import { Kbd } from "@/components/ui/kbd";
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
  const loc = useLocation();
  const pageTitle = (() => {
    const p = loc.pathname;
    if (p.startsWith("/order-masuk")) return "Order masuk";
    if (p.startsWith("/backlog")) return "Backlog";
    if (p.startsWith("/analytics")) return "Analytics";
    if (p.startsWith("/products")) return "Products";
    if (p.startsWith("/batches/")) return "Batch detail";
    return "Orders Ops";
  })();

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
      <header className="sticky top-0 z-40 border-b bg-card/60 backdrop-blur">
        <div className="mx-auto flex h-14 max-w-6xl items-center justify-between gap-4 px-4">
          <Link to="/" className="text-left no-underline">
            <div className="font-heading text-lg font-semibold tracking-tight text-foreground">
              {pageTitle}
            </div>
          </Link>
          <nav className="flex flex-wrap items-center gap-2">
            <NavBtn to="/" end>
              Home
            </NavBtn>
            <NavBtn to="/order-masuk">Order Masuk</NavBtn>
            <NavBtn to="/backlog">Backlog</NavBtn>
            <NavBtn to="/analytics">Analytics</NavBtn>
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
          <Route path="/order-masuk" element={<NewOrdersPage />} />
          <Route path="/backlog" element={<BacklogPage />} />
          <Route path="/analytics" element={<AnalyticsPage />} />
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
  const [stats, setStats] = useState<TodayStats | null>(null);
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
      const [b, list, s] = await Promise.all([
        fetchBacklog(),
        fetchBatchesToday(),
        fetchTodayStats(),
      ]);
      setBacklog(b);
      setBatches(list);
      setStats(s);
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

  const statsDateLabel = useMemo(() => {
    if (!stats) return null;
    return new Date(`${stats.date}T00:00:00+07:00`).toLocaleDateString(
      "id-ID",
      { weekday: "long", day: "numeric", month: "short" },
    );
  }, [stats]);
  const maxCarrier = Math.max(1, ...(stats?.carriers.map((c) => c.count) ?? [1]));

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center justify-end gap-3">
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

      {/* Backlog / urgent / batch — strip compact */}
      <Card>
        <CardPanel className="grid grid-cols-3 divide-x p-0!">
          <Link
            to="/backlog"
            className="no-underline px-5 py-3.5 transition-colors hover:bg-accent/40"
          >
            <p className="text-muted-foreground text-xs">Backlog</p>
            <p className="font-heading text-2xl font-semibold tabular-nums">
              {loading && !backlog ? (
                <Skeleton className="h-7 w-10" />
              ) : (
                backlog?.total ?? 0
              )}
            </p>
            <p className="text-muted-foreground text-[11px]">
              new · belum masuk batch
            </p>
          </Link>
          <div className="px-5 py-3.5">
            <p className="text-muted-foreground text-xs">Urgent</p>
            <p className="font-heading text-2xl font-semibold tabular-nums">
              {loading && !backlog ? (
                <Skeleton className="h-7 w-10" />
              ) : (
                backlog?.urgentCount ?? 0
              )}
            </p>
            <p className="text-muted-foreground text-[11px]">
              instant · sameday · gojek
            </p>
          </div>
          <div className="px-5 py-3.5">
            <p className="text-muted-foreground text-xs">Batch hari ini</p>
            <p className="font-heading text-2xl font-semibold tabular-nums">
              {loading && !batches ? (
                <Skeleton className="h-7 w-10" />
              ) : (
                batches?.batches.length ?? 0
              )}
            </p>
            <p className="text-muted-foreground text-[11px]">
              {batches ? `WIB ${batches.date}` : "Asia/Jakarta"}
            </p>
          </div>
        </CardPanel>
      </Card>

      {/* Hari ini — volume, platform, ekspedisi, top produk */}
      <Card>
        <CardHeader className="pb-3">
          <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
            <CardTitle className="text-base">Hari ini</CardTitle>
            <CardDescription className="capitalize">
              {statsDateLabel ?? "…"} · WIB
            </CardDescription>
          </div>
          <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1.5 pt-1.5">
            {loading && !stats ? (
              <Skeleton className="h-10 w-20" />
            ) : (
              <span className="font-heading text-4xl font-semibold tracking-tight tabular-nums">
                {stats?.totalOrders ?? 0}
              </span>
            )}
            <span className="text-muted-foreground text-sm">
              order · {stats?.totalItems ?? 0} item
            </span>
            {stats?.platforms.map((p) => (
              <span
                key={p.platform}
                className="inline-flex items-center gap-1.5 rounded-md border border-input bg-popover px-2 py-0.5 text-xs font-medium capitalize"
              >
                {p.platform}
                <span className="text-muted-foreground tabular-nums">
                  {p.count}
                </span>
              </span>
            ))}
          </div>
        </CardHeader>
        <CardPanel className="grid gap-x-8 gap-y-5 lg:grid-cols-2">
          <div className="flex min-w-0 flex-col gap-1.5">
            <p className="text-muted-foreground text-xs font-medium tracking-wide uppercase">
              Ekspedisi
            </p>
            {loading && !stats ? (
              <>
                <Skeleton className="h-4 w-full" />
                <Skeleton className="h-4 w-4/5" />
                <Skeleton className="h-4 w-3/5" />
              </>
            ) : !stats?.carriers.length ? (
              <p className="text-muted-foreground text-sm">
                Belum ada order hari ini.
              </p>
            ) : (
              stats.carriers.map((c) => (
                <div key={c.carrier} className="flex items-center gap-2.5 text-sm">
                  <span
                    className="w-24 shrink-0 truncate text-muted-foreground"
                    title={c.carrier}
                  >
                    {c.carrier}
                  </span>
                  <div className="h-1.5 min-w-0 flex-1 overflow-hidden rounded-full bg-muted">
                    <div
                      className="h-full rounded-full bg-primary/70 transition-[width] duration-500"
                      style={{
                        width: `${Math.max(4, (c.count / maxCarrier) * 100)}%`,
                      }}
                    />
                  </div>
                  <span className="w-8 shrink-0 text-right font-medium tabular-nums">
                    {c.count}
                  </span>
                </div>
              ))
            )}
          </div>
          <div className="flex min-w-0 flex-col gap-1.5">
            <p className="text-muted-foreground text-xs font-medium tracking-wide uppercase">
              Top produk
            </p>
            {loading && !stats ? (
              <>
                <Skeleton className="h-4 w-full" />
                <Skeleton className="h-4 w-5/6" />
                <Skeleton className="h-4 w-2/3" />
              </>
            ) : !stats?.topProducts.length ? (
              <p className="text-muted-foreground text-sm">
                Belum ada item terjual hari ini.
              </p>
            ) : (
              stats.topProducts.map((p, i) => (
                <div key={p.sku} className="flex items-center gap-2.5 text-sm">
                  <span className="w-4 shrink-0 text-right text-muted-foreground text-xs tabular-nums">
                    {i + 1}
                  </span>
                  <span
                    className="w-28 shrink-0 truncate font-mono text-xs"
                    title={p.sku}
                  >
                    {p.sku}
                  </span>
                  <span
                    className="min-w-0 flex-1 truncate text-muted-foreground"
                    title={p.name ?? undefined}
                  >
                    {p.name}
                  </span>
                  <span className="shrink-0 font-medium tabular-nums">
                    ×{p.qty}
                  </span>
                </div>
              ))
            )}
          </div>
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

// ---------------------------------------------------------------------------
// Order Masuk — incoming orders feed (state=new)
// ---------------------------------------------------------------------------

const PLATFORM_BADGE: [string, string][] = [
  ["shopee", "bg-orange-500/10 text-orange-700 dark:text-orange-400"],
  ["tiktok", "bg-neutral-800/8 text-neutral-800 dark:bg-neutral-100/12 dark:text-neutral-200"],
  ["tokopedia", "bg-emerald-500/10 text-emerald-700 dark:text-emerald-400"],
  ["lazada", "bg-blue-500/10 text-blue-700 dark:text-blue-400"],
];

function platformBadgeClass(platform: string): string {
  const key = platform.trim().toLowerCase();
  for (const [name, cls] of PLATFORM_BADGE) {
    if (key.includes(name)) return cls;
  }
  return "bg-secondary text-secondary-foreground";
}

function formatWibShort(iso: string | null | undefined): string {
  if (!iso) return "—";
  try {
    return new Intl.DateTimeFormat("en-GB", {
      timeZone: "Asia/Jakarta",
      day: "2-digit",
      month: "short",
      hour: "2-digit",
      minute: "2-digit",
      hour12: false,
    }).format(new Date(iso));
  } catch {
    return iso;
  }
}

function parseMoney(s: string | null | undefined): number | null {
  if (s == null || s.trim() === "") return null;
  const n = Number(s);
  return Number.isFinite(n) ? n : null;
}

function Thumb({ src, alt }: { src: string | null; alt: string }) {
  const [broken, setBroken] = useState(false);
  if (!src || broken) {
    return (
      <div
        className="flex size-12 shrink-0 items-center justify-center rounded-lg border bg-muted/50 text-muted-foreground/60"
        title="Tidak ada gambar"
      >
        <svg
          viewBox="0 0 24 24"
          className="size-5"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden
        >
          <path d="M21 8l-9-5-9 5 9 5 9-5zM3 8v8l9 5 9-5V8" />
        </svg>
      </div>
    );
  }
  return (
    <img
      src={src}
      alt={alt}
      loading="lazy"
      referrerPolicy="no-referrer"
      onError={() => setBroken(true)}
      className="size-12 shrink-0 rounded-lg border object-cover shadow-xs/50 transition-transform duration-200 hover:scale-110 hover:shadow-md"
    />
  );
}

function buyerName(o: NewOrder): string | null {
  return o.buyerUsername?.trim() || o.contactPerson?.trim() || null;
}

/** One table row = one order item, carrying its parent order for context. */
type FeedRow = {
  orderId: number;
  order: NewOrder;
  item: NewOrderItem | null;
};

/** Order-level columns rendered once per contiguous order group (rowSpan). */
const ORDER_COL_IDS = new Set(["select", "status", "platform", "buyer", "ekspedisi"]);

const FEED_CELL_CLASS: Record<string, string> = {
  select: "text-center",
  status: "text-center",
  judul: "max-w-[26rem] whitespace-normal",
  varian: "max-w-[12rem] whitespace-normal",
  buyer: "max-w-[12rem] whitespace-normal",
};

const SESSION_LABEL: Record<BatchSession, string> = {
  morning: "pagi",
  afternoon: "siang",
  urgent: "urgent",
};

const PAGE_SIZE = 50;

const FEED_TABS: { id: FeedStatus; label: string }[] = [
  { id: "new", label: "Baru" },
  { id: "processing", label: "Diproses" },
  { id: "shipped", label: "Dikirim" },
  { id: "completed", label: "Selesai" },
  { id: "all", label: "Semua" },
];

const feedColumns: ColumnDef<FeedRow>[] = [
  {
    id: "status",
    header: "Status",
    accessorFn: (r) => (r.order.summaryPrinted ? 1 : 0),
    cell: ({ row }) => {
      const o = row.original.order;
      if (!o.summaryPrinted) {
        return (
          <span
            className="inline-flex items-center justify-center rounded-md p-1 text-muted-foreground/70 transition-colors hover:text-foreground"
            title="Belum dicetak — eligible untuk batch berikutnya"
          >
            <Printer className="size-4" />
          </span>
        );
      }
      const label = o.batchSession
        ? `Sudah dicetak — batch ${o.batchSession}`
        : "Sudah dicetak di BigSeller (summary list)";
      return (
        <span
          className="inline-flex items-center justify-center rounded-md bg-success/10 p-1 text-success-foreground"
          title={label}
        >
          <BadgeCheck className="size-4" />
        </span>
      );
    },
  },
  {
    id: "gambar",
    header: "Gambar",
    enableSorting: false,
    cell: ({ row }) => (
      <Thumb
        src={row.original.item?.imageUrl ?? null}
        alt={row.original.item?.itemName ?? row.original.order.platformOrderId}
      />
    ),
  },
  {
    id: "judul",
    header: "Judul",
    cell: ({ row }) => {
      const it = row.original.item;
      if (!it) {
        return <span className="text-muted-foreground text-sm">Tanpa item</span>;
      }
      return (
        <div className="flex flex-col gap-0.5 py-0.5">
          <div className="flex items-start gap-1.5">
            <span className="line-clamp-2 font-medium leading-snug">
              {it.itemName?.trim() || it.sku || "—"}
            </span>
            {it.quantity > 1 && (
              <Badge
                variant="secondary"
                size="sm"
                className="mt-0.5 shrink-0 tabular-nums"
              >
                ×{it.quantity}
              </Badge>
            )}
          </div>
          {it.sku && (
            <span className="font-mono text-[11px] text-muted-foreground">
              {it.sku}
            </span>
          )}
        </div>
      );
    },
  },
  {
    id: "varian",
    header: "Varian",
    cell: ({ row }) => (
      <span className="text-muted-foreground text-sm">
        {row.original.item?.variantAttr?.trim() || "—"}
      </span>
    ),
  },
  {
    id: "harga",
    header: "Harga",
    cell: ({ row }) => {
      const it = row.original.item;
      if (!it) return <span className="text-muted-foreground">—</span>;
      const unit = parseMoney(it.unitPrice);
      const line = parseMoney(it.amount) ?? (unit ?? 0) * it.quantity;
      return (
        <div className="flex flex-col items-end gap-0.5">
          <span className="font-medium tabular-nums">{formatIdr(unit)}</span>
          {it.quantity > 1 && (
            <span className="text-[11px] text-muted-foreground tabular-nums">
              {it.quantity} × {formatIdr(unit)} = {formatIdr(line)}
            </span>
          )}
        </div>
      );
    },
  },
  {
    id: "platform",
    header: "Platform",
    cell: ({ row }) => {
      const o = row.original.order;
      return (
        <div className="flex flex-col items-start gap-1 py-0.5">
          <Badge className={cn("capitalize", platformBadgeClass(o.platform))}>
            {o.platform}
          </Badge>
          <span className="font-mono text-[11px] text-muted-foreground">
            {o.platformOrderId}
          </span>
          <span className="text-[11px] text-muted-foreground">
            {formatWibShort(o.orderedAt)} WIB
          </span>
        </div>
      );
    },
  },
  {
    id: "buyer",
    header: "Nama Buyer",
    cell: ({ row }) => {
      const o = row.original.order;
      const qty = o.itemTotalNum ?? o.items.reduce((s, it) => s + it.quantity, 0);
      return (
        <div className="flex flex-col gap-0.5 py-0.5">
          <span className="font-medium leading-snug">{buyerName(o) ?? "—"}</span>
          <span className="text-[11px] text-muted-foreground">
            {qty} barang · {formatIdr(parseMoney(o.amount))}
          </span>
        </div>
      );
    },
  },
  {
    id: "ekspedisi",
    header: "Ekspedisi",
    cell: ({ row }) => {
      const o = row.original.order;
      return (
        <div className="flex flex-col items-start gap-1 py-0.5">
          <span className="text-sm leading-snug">{o.carrier?.trim() || "—"}</span>
          {o.isUrgent && (
            <Badge variant="warning" size="sm">
              Urgent
            </Badge>
          )}
        </div>
      );
    },
  },
];

function NewOrdersPage() {
  const [data, setData] = useState<OrdersFeedResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [autoRefresh, setAutoRefresh] = useState(true);
  const [lastUpdated, setLastUpdated] = useState<Date | null>(null);
  const [flashIds, setFlashIds] = useState<Set<number>>(new Set());
  const knownIds = useRef<Set<number> | null>(null);

  // Bulk selection: order-level (rows are per item, many rows share an order).
  // Full order objects are kept so grouping survives page changes.
  const [selectedOrders, setSelectedOrders] = useState<
    Map<number, NewOrder>
  >(new Map());
  const [printing, setPrinting] = useState<BatchSession | null>(null);
  const [printResult, setPrintResult] = useState<SelectionBatchResult | null>(
    null,
  );
  const [packing, setPacking] = useState(false);
  const [packResult, setPackResult] = useState<PackResult | null>(null);

  const [statusTab, setStatusTab] = useState<FeedStatus>("new");
  const [page, setPage] = useState(0);
  const [searchInput, setSearchInput] = useState("");
  const [appliedQ, setAppliedQ] = useState("");
  const searchRef = useRef<HTMLInputElement>(null);
  const mounted = useRef(false);

  // Debounce the search box into the server query.
  useEffect(() => {
    const t = setTimeout(() => {
      setAppliedQ(searchInput.trim());
      setPage(0);
    }, 350);
    return () => clearTimeout(t);
  }, [searchInput]);

  const load = useCallback(
    async (silent: boolean) => {
      if (!silent) setLoading(true);
      setError(null);
      try {
        const resp = await fetchOrdersFeed({
          status: statusTab,
          q: appliedQ || undefined,
          limit: PAGE_SIZE,
          offset: page * PAGE_SIZE,
        });
        // Flash newly arrived orders (unfiltered new tab only).
        if (statusTab === "new" && appliedQ === "" && page === 0) {
          const ids = resp.orders.map((o) => o.orderId);
          const prev = knownIds.current;
          if (prev) {
            setFlashIds(new Set(ids.filter((id) => !prev.has(id))));
            knownIds.current = new Set([...prev, ...ids]);
          } else {
            setFlashIds(new Set());
            knownIds.current = new Set(ids);
          }
        } else {
          setFlashIds(new Set());
        }
        setData(resp);
        setLastUpdated(new Date());
      } catch (err) {
        setError(err instanceof Error ? err.message : "Failed to load");
      } finally {
        setLoading(false);
      }
    },
    [statusTab, appliedQ, page],
  );

  useEffect(() => {
    // Full skeleton only on first mount; silent refreshes afterwards.
    void load(mounted.current);
    mounted.current = true;
  }, [load]);

  useEffect(() => {
    if (!autoRefresh) return;
    const t = setInterval(() => void load(true), 30_000);
    return () => clearInterval(t);
  }, [autoRefresh, load]);

  // "/" focuses the search field from anywhere on the page.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement | null)?.tagName;
      if (
        e.key === "/" &&
        tag !== "INPUT" &&
        tag !== "TEXTAREA" &&
        tag !== "SELECT"
      ) {
        e.preventDefault();
        searchRef.current?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const orders = useMemo<NewOrder[]>(() => data?.orders ?? [], [data]);
  const counts =
    data?.counts ?? { new: 0, processing: 0, shipped: 0, completed: 0, all: 0 };
  const total = data?.total ?? 0;
  const pageCount = Math.max(1, Math.ceil(total / PAGE_SIZE));
  const rangeStart = total === 0 ? 0 : page * PAGE_SIZE + 1;
  const rangeEnd = Math.min((page + 1) * PAGE_SIZE, total);
  /** Tabs that support selection + bulk actions (Baru: pack/print; Diproses: print resi). */
  const selectable = statusTab === "new" || statusTab === "processing";

  const feedRows = useMemo(() => {
    const rows: FeedRow[] = [];
    for (const o of orders) {
      if (o.items.length === 0) {
        rows.push({ orderId: o.orderId, order: o, item: null });
        continue;
      }
      for (const it of o.items) {
        rows.push({ orderId: o.orderId, order: o, item: it });
      }
    }
    return rows;
  }, [orders]);

  /** Unprinted orders on the current page (for the select-shortcut hint). */
  const pageUnprintedCount = useMemo(
    () => orders.filter((o) => !o.summaryPrinted).length,
    [orders],
  );

  function toggleOrder(order: NewOrder) {
    setSelectedOrders((prev) => {
      const next = new Map(prev);
      if (next.has(order.orderId)) next.delete(order.orderId);
      else next.set(order.orderId, order);
      return next;
    });
  }

  function selectAllUnprinted() {
    setSelectedOrders((prev) => {
      const next = new Map(prev);
      for (const o of orders) if (!o.summaryPrinted) next.set(o.orderId, o);
      return next;
    });
  }

  /** Distinct order ids on the current page (for select-all + rowSpan). */
  const pageOrderIds = useMemo(() => {
    const ids: number[] = [];
    const seen = new Set<number>();
    for (const r of feedRows) {
      if (!seen.has(r.orderId)) {
        seen.add(r.orderId);
        ids.push(r.orderId);
      }
    }
    return ids;
  }, [feedRows]);

  const pageAllSelected =
    pageOrderIds.length > 0 &&
    pageOrderIds.every((id) => selectedOrders.has(id));
  const pageSomeSelected = pageOrderIds.some((id) => selectedOrders.has(id));

  function togglePageOrders() {
    setSelectedOrders((prev) => {
      const next = new Map(prev);
      if (pageAllSelected) for (const o of orders) next.delete(o.orderId);
      else for (const o of orders) next.set(o.orderId, o);
      return next;
    });
  }

  /** Selected orders that are not printed yet — only these can be claimed
   *  into a print batch. Pack, on the other hand, accepts the full selection. */
  const selectedUnprintedIds = useMemo(
    () =>
      [...selectedOrders.values()]
        .filter((o) => !o.summaryPrinted)
        .map((o) => o.orderId),
    [selectedOrders],
  );

  async function printSelected(session: BatchSession) {
    if (selectedUnprintedIds.length === 0 || printing) return;
    setPrinting(session);
    setError(null);
    try {
      const result = await createBatchFromSelection(session, selectedUnprintedIds);
      setPrintResult(result);
      setSelectedOrders(new Map());
    } catch (err) {
      setError(err instanceof Error ? err.message : "Gagal membuat batch");
    } finally {
      setPrinting(null);
    }
  }

  async function packSelected() {
    const ids = [...selectedOrders.keys()];
    if (ids.length === 0 || packing) return;
    setPacking(true);
    setError(null);
    try {
      const result = await packOrders(ids);
      setPackResult(result);
      setSelectedOrders(new Map());
      // The server refreshes order states in the background; catch up soon.
      setTimeout(() => void load(true), 6000);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Gagal pack order");
    } finally {
      setPacking(false);
    }
  }

  // --- Print resi (shipping labels) via BigSeller, grouped per marketplace
  // --- + carrier (fallback: manual copy per group).
  const [resiOpen, setResiOpen] = useState(false);
  const [copiedKey, setCopiedKey] = useState<string | null>(null);
  const [pdfBusy, setPdfBusy] = useState(false);

  // --- Bulk print resi through the BigSeller print plugin. The backend
  // --- buffers labels in BigSeller (checkPrintInfo) and hands us the puid;
  // --- the browser then drives the local plugin over ws://localhost:21319
  // --- (setPuid → the plugin pulls the labels and prints them itself).
  type ResiPhase = "preparing" | "connecting" | "handshake" | "printing" | "error";
  interface ResiPrintState {
    phase: ResiPhase;
    prep: ResiPrep | null;
    orderIds: number[];
    confirmed: boolean;
    /** wavePrintV2 (the actual print command) sent once. */
    sentWave: boolean;
    printers: string[];
    printer: string | null;
    log: string[];
    error: string | null;
  }
  const [resiPrint, setResiPrint] = useState<ResiPrintState | null>(null);
  const resiWsRef = useRef<WebSocket | null>(null);

  function startResiPrint() {
    const ids = [...selectedOrders.keys()];
    if (ids.length === 0) return;
    setResiPrint({
      phase: "preparing",
      prep: null,
      orderIds: ids,
      confirmed: false,
      sentWave: false,
      printers: [],
      printer: null,
      log: [],
      error: null,
    });
    void (async () => {
      let prep: ResiPrep;
      try {
        prep = await prepareResiPrint(ids);
      } catch (err) {
        setResiPrint(
          (p) =>
            p && {
              ...p,
              phase: "error",
              error: err instanceof Error ? err.message : "Gagal menyiapkan print resi",
            },
        );
        return;
      }
      setResiPrint((p) => p && { ...p, phase: "connecting", prep });
      let ws: WebSocket;
      try {
        ws = new WebSocket("ws://localhost:21319");
      } catch {
        setResiPrint(
          (p) => p && { ...p, phase: "error", error: "Plugin BigSeller tidak terdeteksi." },
        );
        return;
      }
      resiWsRef.current = ws;
      const failTimer = window.setTimeout(() => {
        setResiPrint(
          (p) =>
            p && (p.phase === "connecting" || p.phase === "handshake")
              ? {
                  ...p,
                  phase: "error",
                  error:
                    "Tidak terhubung ke plugin BigSeller (ws://localhost:21319). Pastikan BigSeller Print Plugin berjalan di mesin ini.",
                }
              : p,
        );
        try {
          ws.close();
        } catch {
          /* already closed */
        }
      }, 6000);
      ws.onopen = () => {
        window.clearTimeout(failTimer);
        setResiPrint(
          (p) => p && { ...p, phase: "handshake", log: [...p.log, "Terhubung ke plugin print"] },
        );
        ws.send(JSON.stringify({ method: "getPrinter", params: null }));
      };
      ws.onerror = () => {
        window.clearTimeout(failTimer);
        setResiPrint(
          (p) =>
            p && p.phase !== "printing"
              ? {
                  ...p,
                  phase: "error",
                  error:
                    "Koneksi ke plugin gagal. Pastikan BigSeller Print Plugin berjalan di mesin ini.",
                }
              : p,
        );
      };
      ws.onmessage = (m) => {
        let msg: { method?: string; message?: string | null; data?: unknown; code?: string };
        try {
          msg = JSON.parse(String(m.data));
        } catch {
          return;
        }
        setResiPrint((p) => {
          if (!p) return p;
          const detail = typeof msg.data === "string" ? msg.data : "";
          const log = [
            ...p.log,
            [msg.method, msg.code ?? "", detail].filter(Boolean).join(" · "),
          ];
          if (msg.code === "FAIL") {
            return { ...p, phase: "error", error: msg.message || "Plugin melaporkan kegagalan", log };
          }
          switch (msg.method) {
            case "getPrinter": {
              // SDK handshake order: printer list -> setPuid (who am I) ->
              // getVersion -> changePrinter (which printer); after that the
              // plugin pulls the labels buffered by checkPrintInfo and prints.
              const printers = Array.isArray(msg.data) ? (msg.data as string[]) : [];
              const printer =
                (typeof msg.message === "string" && msg.message) || printers[0] || null;
              if (p.prep) {
                ws.send(JSON.stringify({ method: "setPuid", params: [p.prep.encryptId, p.prep.uid] }));
              } else {
                ws.send(JSON.stringify({ method: "getVersion" }));
              }
              return { ...p, printers, printer, log };
            }
            case "setPuid":
              ws.send(JSON.stringify({ method: "getVersion" }));
              return { ...p, log };
            case "getVersion":
              // Confirm the target printer — without this the plugin prints
              // to whatever its current default is (e.g. Adobe PDF). The
              // plugin's changePrinterResponse is what triggers the actual
              // print command (handled below).
              if (p.printer) {
                ws.send(JSON.stringify({ method: "changePrinter", params: [p.printer] }));
              }
              return {
                ...p,
                phase: "printing",
                log: [...log, `Plugin v${detail} — printer "${p.printer ?? "?"}"`],
              };
            case "changePrinterResponse": {
              // The real print command (captured from BigSeller's print
              // page): wavePrintV2 with the internal order ids. The plugin
              // fetches each label from BigSeller and prints it. Plugin
              // >= 1.2.2 uses wavePrintV2 (ours reports 1.2.5.9).
              if (!p.sentWave && p.prep && p.prep.labels.length > 0) {
                const orderList = p.prep.labels.map((l) => ({
                  id: l.orderId,
                  isSelf: false,
                  isLazada: l.platform === "lazada",
                  printSource: 1, // "processing"
                  printBatchNo: "",
                }));
                ws.send(JSON.stringify({ method: "wavePrintV2", params: [{ orderList }] }));
                // Mark the labels printed in BigSeller (confirmLabelPrint).
                if (!p.confirmed) {
                  void confirmResiPrint(p.orderIds).catch(() => undefined);
                }
              }
              return {
                ...p,
                sentWave: true,
                confirmed: true,
                log: [
                  ...log,
                  `wavePrintV2 terkirim (${p.prep?.labels.length ?? 0} order) — plugin mencetak…`,
                ],
              };
            }
            case "printProcess":
              return { ...p, log: [...log, "progress: " + JSON.stringify(msg.data).slice(0, 140)] };
            default:
              return { ...p, log };
          }
        });
      };
    })();
  }

  function closeResiPrint() {
    try {
      resiWsRef.current?.close();
    } catch {
      /* already closed */
    }
    resiWsRef.current = null;
    setResiPrint(null);
    void load(true);
  }

  // --- On-demand BigSeller sync (Refresh button → progress dialog) ---
  const [syncOpen, setSyncOpen] = useState(false);
  const [sync, setSync] = useState<SyncProgress | null>(null);

  async function startSync() {
    setError(null);
    setSync(null);
    setSyncOpen(true);
    try {
      await startSyncRun();
    } catch (err) {
      setError(
        err instanceof Error ? err.message : "Gagal memulai sinkronisasi",
      );
      setSyncOpen(false);
    }
  }

  // Poll the run while the dialog is open; stop once it finishes.
  useEffect(() => {
    if (!syncOpen) return;
    let cancelled = false;
    let timer = 0;
    const tick = async () => {
      try {
        const p = await fetchSyncProgress();
        if (cancelled) return;
        setSync(p);
        if (!p.running) return;
      } catch {
        // transient hiccup — keep polling
      }
      if (!cancelled) timer = window.setTimeout(tick, 600);
    };
    void tick();
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [syncOpen]);

  async function downloadSelectedPdf() {
    const ids = [...selectedOrders.keys()];
    if (ids.length === 0 || pdfBusy) return;
    setPdfBusy(true);
    setError(null);
    try {
      await downloadPicklistPdf(ids);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Gagal membuat PDF");
    } finally {
      setPdfBusy(false);
    }
  }

  const resiGroups = useMemo(() => {
    const m = new Map<string, { platform: string; carrier: string; ids: string[] }>();
    for (const o of selectedOrders.values()) {
      const carrier = o.carrier?.trim() || "Lainnya";
      const key = `${o.platform}|${carrier}`;
      const g = m.get(key) ?? { platform: o.platform, carrier, ids: [] };
      g.ids.push(o.platformOrderId);
      m.set(key, g);
    }
    return [...m.values()].sort((a, b) => b.ids.length - a.ids.length);
  }, [selectedOrders]);

  async function copyGroup(g: { platform: string; carrier: string; ids: string[] }) {
    const key = `${g.platform}|${g.carrier}`;
    const text = g.ids.join(" ");
    try {
      await navigator.clipboard.writeText(text);
      setCopiedKey(key);
      setTimeout(() => setCopiedKey((c) => (c === key ? null : c)), 2000);
    } catch {
      window.prompt("Salin manual:", text);
    }
  }

  const selectColumn: ColumnDef<FeedRow> = {
    id: "select",
    header: () => (
      <Checkbox
        checked={pageAllSelected}
        indeterminate={!pageAllSelected && pageSomeSelected}
        disabled={pageOrderIds.length === 0}
        onCheckedChange={() => togglePageOrders()}
        aria-label="Pilih semua order di halaman ini"
      />
    ),
    cell: ({ row }) => (
      <Checkbox
        checked={selectedOrders.has(row.original.orderId)}
        onCheckedChange={() => toggleOrder(row.original.order)}
        aria-label={`Pilih ${row.original.order.platformOrderId}`}
      />
    ),
    enableSorting: false,
  };

  const table = useReactTable({
    data: feedRows,
    columns: selectable ? [selectColumn, ...feedColumns] : feedColumns,
    getCoreRowModel: getCoreRowModel(),
  });

  const visibleRows = table.getRowModel().rows;

  // Contiguous same-order runs → rowSpan for the order-level columns.
  const groupInfo = useMemo(() => {
    const info: { start: boolean; span: number }[] = [];
    for (let i = 0; i < visibleRows.length; i++) {
      const prevId = i > 0 ? visibleRows[i - 1].original.orderId : null;
      if (visibleRows[i].original.orderId !== prevId) {
        let span = 1;
        while (
          i + span < visibleRows.length &&
          visibleRows[i + span].original.orderId ===
            visibleRows[i].original.orderId
        ) {
          span++;
        }
        info.push({ start: true, span });
      } else {
        info.push({ start: false, span: 0 });
      }
    }
    return info;
  }, [visibleRows]);

  return (
    <div className="flex flex-col gap-4">
      {/* -mt-6 cancels main's top padding so the bar hugs the header even
          before any scrolling; sticky keeps it there afterwards. */}
      <div className="sticky top-14 z-30 -mx-4 -mt-6 border-b bg-card/60 px-4 py-1.5 backdrop-blur">
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1.5">
        <div className="flex flex-wrap items-center gap-1.5">
          {FEED_TABS.map((t) => (
            <button
              key={t.id}
              type="button"
              onClick={() => {
                setStatusTab(t.id);
                setPage(0);
                setSelectedOrders(new Map());
              }}
              className={cn(
                "inline-flex h-7 cursor-pointer items-center gap-1.5 rounded-lg border px-2.5 text-xs font-medium transition-all duration-150 active:scale-[0.97]",
                statusTab === t.id
                  ? "border-primary bg-primary text-primary-foreground shadow-xs"
                  : "border-input bg-popover text-foreground hover:bg-accent/50",
              )}
            >
              {t.label}
              <span
                className={cn(
                  "tabular-nums",
                  statusTab === t.id ? "opacity-70" : "text-muted-foreground",
                )}
              >
                {counts[t.id]}
              </span>
            </button>
          ))}
        </div>

        <div className="relative ms-auto w-full sm:w-64">
          <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            ref={searchRef}
            value={searchInput}
            onChange={(e) => setSearchInput(e.target.value)}
            placeholder="Cari nomor pesanan / buyer…"
            className="pl-8 pr-9 text-sm"
          />
          <Kbd className="absolute top-1/2 right-2 -translate-y-1/2">/</Kbd>
        </div>

          {lastUpdated && (
            <span className="hidden text-muted-foreground text-xs sm:inline">
              Diperbarui{" "}
              {lastUpdated.toLocaleTimeString("id-ID", {
                timeZone: "Asia/Jakarta",
              })}{" "}
              WIB
            </span>
          )}
          <button
            type="button"
            onClick={() => setAutoRefresh((v) => !v)}
            className={cn(
              "inline-flex h-7 cursor-pointer items-center gap-1.5 rounded-lg border px-2.5 text-xs font-medium transition-colors",
              autoRefresh
                ? "border-success/40 bg-success/8 text-success-foreground"
                : "border-input bg-popover text-muted-foreground hover:bg-accent/50",
            )}
            title="Refresh otomatis tiap 30 detik"
          >
            <span className="relative flex size-1.5">
              {autoRefresh && (
                <span className="absolute inline-flex size-full animate-ping rounded-full bg-success opacity-60" />
              )}
              <span className="relative inline-flex size-1.5 rounded-full bg-current" />
            </span>
            Live · 30 dtk
          </button>
          <Button
            variant="outline"
            size="sm"
            onClick={() => void startSync()}
          >
            Refresh
          </Button>
        </div>
      </div>

      {error && (
        <p className="text-destructive text-sm" role="alert">
          {error}
        </p>
      )}

      {loading ? (
        <div className="flex flex-col gap-2">
          <Skeleton className="h-16 w-full" />
          <Skeleton className="h-16 w-full" />
          <Skeleton className="h-16 w-full" />
          <Skeleton className="h-16 w-full" />
        </div>
      ) : orders.length === 0 ? (
        <Empty className="py-16">
          {appliedQ ? (
            <>
              <p className="font-medium">Tidak ada hasil</p>
              <p className="text-muted-foreground text-sm">
                Tidak ada order cocok dengan “{appliedQ}” di tab ini.
              </p>
              <Button
                size="sm"
                variant="outline"
                className="mt-2"
                onClick={() => setSearchInput("")}
              >
                Hapus pencarian
              </Button>
            </>
          ) : (
            <>
              <p className="font-medium">
                {statusTab === "new"
                  ? "Belum ada order baru"
                  : "Tidak ada order di tab ini"}
              </p>
              <p className="text-muted-foreground text-sm">
                {statusTab === "new"
                  ? "Order baru muncul otomatis setelah sync (~60 detik)."
                  : "Order berpindah tab mengikuti statusnya di BigSeller."}
              </p>
            </>
          )}
        </Empty>
      ) : (
        <>
          <Table variant="card">
            <TableHeader>
              {table.getHeaderGroups().map((headerGroup) => (
                <TableRow key={headerGroup.id}>
                  {headerGroup.headers.map((header) => (
                    <TableHead
                      key={header.id}
                      className={cn(
                        header.column.id === "select" && "w-10 text-center",
                        header.column.id === "status" && "w-14 text-center",
                        header.column.id === "gambar" && "w-16",
                        header.column.id === "harga" && "text-right",
                      )}
                    >
                      {flexRender(
                        header.column.columnDef.header,
                        header.getContext(),
                      )}
                    </TableHead>
                  ))}
                </TableRow>
              ))}
            </TableHeader>
            <TableBody>
              {visibleRows.length === 0 ? (
                <TableRow>
                  <TableCell
                    colSpan={selectable ? 8 : 7}
                    className="h-28 text-center whitespace-normal"
                  >
                    <p className="text-sm">Tidak ada order di halaman ini.</p>
                  </TableCell>
                </TableRow>
              ) : (
                visibleRows.map((row, i) => (
                  <TableRow
                    key={row.id}
                    className={
                      flashIds.has(row.original.orderId)
                        ? "[&>td]:animate-row-flash"
                        : undefined
                    }
                  >
                    {row.getVisibleCells().map((cell) => {
                      const isOrderCol = ORDER_COL_IDS.has(cell.column.id);
                      const group = groupInfo[i];
                      if (isOrderCol && !group.start) return null;
                      return (
                        <TableCell
                          key={cell.id}
                          rowSpan={isOrderCol ? group.span : undefined}
                          className={cn(
                            isOrderCol &&
                              (cell.column.id === "platform" ||
                              cell.column.id === "buyer" ||
                              cell.column.id === "ekspedisi"
                                ? "align-top"
                                : "align-middle"),
                            FEED_CELL_CLASS[cell.column.id],
                          )}
                        >
                          {flexRender(
                            cell.column.columnDef.cell,
                            cell.getContext(),
                          )}
                        </TableCell>
                      );
                    })}
                  </TableRow>
                ))
              )}
            </TableBody>
          </Table>

          <div className="flex flex-wrap items-center justify-between gap-3">
            <p className="text-muted-foreground text-xs">
              Item{" "}
              <span className="font-medium text-foreground tabular-nums">
                {rangeStart}
              </span>
              –
              <span className="font-medium text-foreground tabular-nums">
                {rangeEnd}
              </span>{" "}
              dari{" "}
              <span className="font-medium text-foreground tabular-nums">
                {total}
              </span>
            </p>
            <div className="flex items-center gap-1">
              <Button
                size="xs"
                variant="outline"
                disabled={page === 0}
                onClick={() => setPage((p) => p - 1)}
                aria-label="Halaman sebelumnya"
              >
                <ChevronLeft className="size-3.5" />
              </Button>
              <span className="px-1.5 text-muted-foreground text-xs tabular-nums">
                {page + 1} / {pageCount}
              </span>
              <Button
                size="xs"
                variant="outline"
                disabled={page >= pageCount - 1}
                onClick={() => setPage((p) => p + 1)}
                aria-label="Halaman berikutnya"
              >
                <ChevronRight className="size-3.5" />
              </Button>
            </div>
          </div>
        </>
      )}

      {selectable && selectedOrders.size > 0 && (
        <div className="pointer-events-none fixed inset-x-0 bottom-4 z-50 flex justify-center px-4">
          <div className="pointer-events-auto flex flex-wrap items-center gap-2 rounded-2xl border bg-popover/95 py-2 pe-2 ps-4 shadow-xl backdrop-blur animate-bar-in">
            <div className="flex flex-col">
              <span className="text-sm font-semibold leading-tight tabular-nums">
                {selectedOrders.size} order dipilih
              </span>
              {statusTab === "new" ? (
                pageUnprintedCount > selectedUnprintedIds.length ? (
                  <button
                    type="button"
                    onClick={selectAllUnprinted}
                    className="cursor-pointer text-start text-[11px] text-info-foreground hover:underline"
                  >
                    pilih semua {pageUnprintedCount} yang belum cetak (halaman
                    ini)
                  </button>
                ) : (
                  <span className="text-[11px] leading-tight text-muted-foreground">
                    Summary List PDF · klaim sekali, anti double print
                  </span>
                )
              ) : (
                <span className="text-[11px] leading-tight text-muted-foreground">
                  Print resi dikelompokkan per marketplace + ekspedisi
                </span>
              )}
            </div>
            <div className="mx-1 h-8 w-px bg-border" />
            <Button
              size="sm"
              variant="outline"
              loading={pdfBusy}
              disabled={pdfBusy || packing || printing !== null}
              onClick={() => void downloadSelectedPdf()}
              title="Satu PDF pick list untuk semua order yang dipilih — tanpa klaim batch"
            >
              <Printer className="size-3.5" /> PDF ({selectedOrders.size})
            </Button>
            {statusTab === "new" ? (
              <>
                <Button
                  size="sm"
                  loading={packing}
                  disabled={packing || printing !== null}
                  onClick={() => void packSelected()}
                  title="Pack order di BigSeller (pindah ke In Process)"
                >
                  <PackageCheck className="size-3.5" /> Pack
                </Button>
                <Button
                  size="sm"
                  variant="secondary"
                  loading={printing === "morning"}
                  disabled={printing !== null || packing || selectedUnprintedIds.length === 0}
                  onClick={() => void printSelected("morning")}
                  title={
                    selectedUnprintedIds.length === 0
                      ? "Hanya order yang belum dicetak yang bisa di-print"
                      : "Klaim + Summary List PDF"
                  }
                >
                  <Printer className="size-3.5" /> Print pagi
                </Button>
                <Button
                  size="sm"
                  variant="secondary"
                  loading={printing === "afternoon"}
                  disabled={printing !== null || packing || selectedUnprintedIds.length === 0}
                  onClick={() => void printSelected("afternoon")}
                  title={
                    selectedUnprintedIds.length === 0
                      ? "Hanya order yang belum dicetak yang bisa di-print"
                      : "Klaim + Summary List PDF"
                  }
                >
                  <Printer className="size-3.5" /> Print siang
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  loading={printing === "urgent"}
                  disabled={printing !== null || packing || selectedUnprintedIds.length === 0}
                  onClick={() => void printSelected("urgent")}
                >
                  <Zap className="size-3.5" /> Urgent
                </Button>
              </>
            ) : (
              <>
                <Button
                  size="sm"
                  disabled={packing || printing !== null}
                  onClick={() => startResiPrint()}
                  title="Cetak resi lewat BigSeller Print Plugin (plugin harus berjalan di mesin ini)"
                >
                  <Printer className="size-3.5" /> Print resi (
                  {selectedOrders.size})
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => {
                    setCopiedKey(null);
                    setResiOpen(true);
                  }}
                  title="Fallback: salin nomor per grup untuk print manual di BigSeller"
                >
                  Salin resi ({selectedOrders.size})
                </Button>
              </>
            )}
            <Button
              size="sm"
              variant="ghost"
              disabled={printing !== null || packing}
              onClick={() => setSelectedOrders(new Map())}
              aria-label="Batal pilih"
            >
              <X className="size-3.5" />
            </Button>
          </div>
        </div>
      )}

      <Dialog
        open={printResult !== null}
        onOpenChange={(open) => {
          if (!open && printing === null) {
            setPrintResult(null);
            void load(true);
          }
        }}
      >
        <DialogPopup>
          <DialogHeader>
            <DialogTitle>
              Batch{" "}
              {printResult
                ? (SESSION_LABEL[printResult.session as BatchSession] ??
                  printResult.session)
                : ""}{" "}
              dibuat
            </DialogTitle>
            <DialogDescription>
              {printResult?.orderCount} order diklaim — Summary List PDF siap
              diunduh. Order yang sudah diklaim tidak bisa masuk batch lain.
            </DialogDescription>
          </DialogHeader>
          {printResult && printResult.skipped.length > 0 && (
            <div className="rounded-lg border border-warning/30 bg-warning/8 px-3 py-2">
              <p className="font-medium text-sm text-warning-foreground">
                {printResult.skipped.length} order dilewati
              </p>
              <ul className="mt-1 list-disc ps-4 text-muted-foreground text-xs">
                {printResult.skipped.slice(0, 5).map((s) => (
                  <li key={s.orderId}>
                    <span className="font-mono">
                      {s.platformOrderId ?? s.orderId}
                    </span>{" "}
                    — {s.reason}
                  </li>
                ))}
                {printResult.skipped.length > 5 && (
                  <li>…dan {printResult.skipped.length - 5} lainnya</li>
                )}
              </ul>
            </div>
          )}
          <DialogFooter>
            <DialogClose render={<Button variant="outline" />}>Tutup</DialogClose>
            <Button render={<Link to={`/batches/${printResult?.id ?? ""}`} />}>
              Buka batch + PDF
            </Button>
          </DialogFooter>
        </DialogPopup>
      </Dialog>

      <Dialog
        open={packResult !== null}
        onOpenChange={(open) => {
          if (!open && !packing) {
            setPackResult(null);
            void load(true);
          }
        }}
      >
        <DialogPopup>
          <DialogHeader>
            <DialogTitle>
              {packResult?.ok ? "Order di-pack" : "Pack gagal"}
            </DialogTitle>
            <DialogDescription>
              {packResult
                ? `${packResult.packed.length} order dikirim ke BigSeller untuk di-pack${
                    packResult.skipped.length > 0
                      ? ` · ${packResult.skipped.length} dilewati (sudah pindah status)`
                      : ""
                  }. Order akan hilang dari daftar dalam beberapa detik.`
                : ""}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <DialogClose render={<Button />}>Tutup</DialogClose>
          </DialogFooter>
        </DialogPopup>
      </Dialog>

      <Dialog
        open={resiOpen}
        onOpenChange={(open) => {
          if (!open) setResiOpen(false);
        }}
      >
        <DialogPopup>
          <DialogHeader>
            <DialogTitle>
              Print resi — {selectedOrders.size} order · {resiGroups.length}{" "}
              grup
            </DialogTitle>
            <DialogDescription>
              BigSeller hanya bisa bulk print per marketplace + ekspedisi.
              Salin nomor per grup, paste di kotak pencarian BigSeller, pilih
              semua, lalu Bulk Print — plugin cetak jalan seperti biasa.
            </DialogDescription>
          </DialogHeader>
          <div className="flex flex-col gap-2">
            {resiGroups.map((g) => {
              const key = `${g.platform}|${g.carrier}`;
              return (
                <div
                  key={key}
                  className="flex items-center justify-between gap-3 rounded-lg border px-3 py-2"
                >
                  <div className="flex flex-wrap items-center gap-2">
                    <Badge
                      className={cn("capitalize", platformBadgeClass(g.platform))}
                    >
                      {g.platform}
                    </Badge>
                    <span className="text-sm">{g.carrier}</span>
                    <span className="text-muted-foreground text-xs tabular-nums">
                      {g.ids.length} order
                    </span>
                  </div>
                  <Button
                    size="xs"
                    variant={copiedKey === key ? "secondary" : "outline"}
                    onClick={() => void copyGroup(g)}
                  >
                    {copiedKey === key ? "Tersalin ✓" : "Salin nomor"}
                  </Button>
                </div>
              );
            })}
          </div>
          <DialogFooter>
            <DialogClose render={<Button variant="outline" />}>Tutup</DialogClose>
            <Button
              render={
                <a
                  href="https://www.bigseller.com/web/order/index.htm?status=processing"
                  target="_blank"
                  rel="noreferrer"
                />
              }
            >
              Buka BigSeller
            </Button>
          </DialogFooter>
        </DialogPopup>
      </Dialog>

      <Dialog
        open={resiPrint !== null}
        onOpenChange={(open) => {
          if (!open) closeResiPrint();
        }}
      >
        <DialogPopup>
          <DialogHeader>
            <DialogTitle>
              Print resi —{" "}
              {resiPrint?.prep
                ? `${resiPrint.prep.labels.length} label`
                : `${selectedOrders.size} order`}
            </DialogTitle>
            <DialogDescription>
              Lewat BigSeller Print Plugin (ws://localhost:21319) — plugin
              harus sedang berjalan di mesin ini.
            </DialogDescription>
          </DialogHeader>
          {resiPrint && (
            <div className="flex flex-col gap-3 pb-2">
              {resiPrint.error ? (
                <p className="rounded-lg border border-destructive/30 bg-destructive/8 px-3 py-2 text-destructive-foreground text-sm">
                  {resiPrint.error}
                </p>
              ) : (
                <div className="flex items-center gap-2 text-sm">
                  <LoaderCircle className="size-4 animate-spin text-primary" />
                  {resiPrint.phase === "preparing" &&
                    "Menyiapkan label di server BigSeller…"}
                  {resiPrint.phase === "connecting" &&
                    "Menghubungkan ke plugin print…"}
                  {resiPrint.phase === "handshake" && "Handshake plugin…"}
                  {resiPrint.phase === "printing" &&
                    "Plugin sedang mengambil label & mencetak resi…"}
                </div>
              )}
              {resiPrint.printers.length > 0 && (
                <label className="flex items-center gap-2 text-muted-foreground text-xs">
                  Printer:
                  <select
                    className="h-8 rounded-lg border border-input bg-popover px-2 text-foreground text-sm"
                    value={resiPrint.printer ?? ""}
                    onChange={(e) => {
                      const name = e.target.value;
                      setResiPrint((p) => p && { ...p, printer: name });
                      try {
                        resiWsRef.current?.send(
                          JSON.stringify({ method: "changePrinter", params: [name] }),
                        );
                      } catch {
                        /* socket closed */
                      }
                    }}
                  >
                    {resiPrint.printers.map((pr) => (
                      <option key={pr} value={pr}>
                        {pr}
                      </option>
                    ))}
                  </select>
                </label>
              )}
              {resiPrint.prep && resiPrint.prep.notPrintable.length > 0 && (
                <p className="text-muted-foreground text-xs">
                  {resiPrint.prep.notPrintable.length} order tidak bisa dicetak
                  (dibatalkan / tidak lagi printable).
                </p>
              )}
              {resiPrint.log.length > 0 && (
                <div className="max-h-36 overflow-y-auto rounded-lg border bg-muted/40 px-3 py-2 font-mono text-[11px] text-muted-foreground">
                  {resiPrint.log.map((l, i) => (
                    <div key={i}>{l}</div>
                  ))}
                </div>
              )}
            </div>
          )}
          <DialogFooter>
            <Button onClick={() => closeResiPrint()}>
              {resiPrint?.phase === "printing" ? "Tutup (cetak jalan terus)" : "Tutup"}
            </Button>
          </DialogFooter>
        </DialogPopup>
      </Dialog>

      <Dialog
        open={syncOpen}
        onOpenChange={(open) => {
          if (!open) {
            setSyncOpen(false);
            // Finished run: refresh the table to show the new state.
            if (sync && !sync.running) void load(true);
          }
        }}
      >
        <DialogPopup>
          <DialogHeader>
            <DialogTitle>Sinkronisasi BigSeller</DialogTitle>
            <DialogDescription>
              Tarik order masuk terbaru langsung dari BigSeller, lalu pulihkan
              state order yang berubah di luar siklus worker.
            </DialogDescription>
          </DialogHeader>
          <div className="flex flex-col gap-0.5 px-4">
            {sync === null && (
              <div className="flex items-center gap-2.5 px-2 py-1.5 text-muted-foreground text-sm">
                <LoaderCircle className="size-4 animate-spin" />
                Memulai…
              </div>
            )}
            {sync?.steps.map((s) => (
              <div
                key={s.key}
                className="flex items-start gap-2.5 rounded-lg px-2 py-1.5"
              >
                <SyncStepIcon state={s.state} />
                <div className="min-w-0">
                  <div
                    className={cn(
                      "text-sm font-medium",
                      s.state === "pending" && "text-muted-foreground",
                    )}
                  >
                    {s.label}
                  </div>
                  {s.detail && (
                    <div
                      className={cn(
                        "text-xs",
                        s.state === "error"
                          ? "text-destructive"
                          : "text-muted-foreground",
                      )}
                    >
                      {s.detail}
                    </div>
                  )}
                </div>
              </div>
            ))}
            {sync && !sync.running && sync.ok === false && sync.error && (
              <p className="mx-2 my-1.5 rounded-lg border border-destructive/30 bg-destructive/8 px-3 py-2 text-destructive-foreground text-sm">
                {sync.error}
              </p>
            )}
          </div>
          <DialogFooter>
            {sync && !sync.running && (
              <Button variant="outline" onClick={() => void startSync()}>
                Jalankan lagi
              </Button>
            )}
            <DialogClose render={<Button />}>
              {sync?.running ? "Tutup (jalan terus)" : "Tutup"}
            </DialogClose>
          </DialogFooter>
        </DialogPopup>
      </Dialog>
    </div>
  );
}

function SyncStepIcon({ state }: { state: SyncStepState }) {
  switch (state) {
    case "running":
      return (
        <LoaderCircle className="mt-0.5 size-4 shrink-0 animate-spin text-primary" />
      );
    case "ok":
      return <CircleCheck className="mt-0.5 size-4 shrink-0 text-success" />;
    case "error":
      return <CircleX className="mt-0.5 size-4 shrink-0 text-destructive" />;
    default:
      return (
        <Circle className="mt-0.5 size-4 shrink-0 text-muted-foreground/40" />
      );
  }
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

const ANALYTICS_PERIODS = [7, 30, 90, 365];

function formatIdrCompact(v: number): string {
  if (!Number.isFinite(v)) return "—";
  if (v >= 1e9)
    return `Rp ${(v / 1e9).toLocaleString("id-ID", { maximumFractionDigits: 2 })} M`;
  if (v >= 1e6)
    return `Rp ${(v / 1e6).toLocaleString("id-ID", { maximumFractionDigits: 1 })} jt`;
  if (v >= 1e3)
    return `Rp ${(v / 1e3).toLocaleString("id-ID", { maximumFractionDigits: 0 })} rb`;
  return formatIdr(v);
}

function Kpi({
  label,
  value,
  sub,
  tone,
}: {
  label: string;
  value: string | null;
  sub?: string;
  tone?: "good" | "bad";
}) {
  return (
    <div className="px-5 py-4">
      <p className="text-muted-foreground text-xs">{label}</p>
      <div
        className={cn(
          "font-heading text-2xl font-semibold tracking-tight tabular-nums",
          tone === "good" && "text-success-foreground",
          tone === "bad" && "text-destructive",
        )}
      >
        {value === null ? <Skeleton className="h-7 w-20" /> : value}
      </div>
      {sub && <p className="text-muted-foreground text-[11px]">{sub}</p>}
    </div>
  );
}

function AnalyticsPage() {
  const [days, setDays] = useState(30);
  const [data, setData] = useState<Analytics | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [topTab, setTopTab] = useState<"revenue" | "margin">("revenue");

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    fetchAnalytics(days)
      .then((d) => {
        if (!cancelled) setData(d);
      })
      .catch((err) => {
        if (!cancelled)
          setError(err instanceof Error ? err.message : "Failed to load");
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [days]);

  const t = data?.totals;
  const maxRev = Math.max(
    1,
    ...(data?.daily.map((d) => parseFloat(d.revenue) || 0) ?? [1]),
  );
  const maxCarrier = Math.max(1, ...(data?.carriers.map((c) => c.count) ?? [1]));
  const totalPlatformRev =
    data?.platforms.reduce((s, p) => s + (parseFloat(p.revenue) || 0), 0) ?? 0;
  const topProducts: AnalyticsProduct[] =
    (topTab === "revenue" ? data?.topRevenue : data?.topMargin) ?? [];

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-1.5">
          {ANALYTICS_PERIODS.map((p) => (
            <button
              key={p}
              type="button"
              onClick={() => setDays(p)}
              className={cn(
                "inline-flex h-7 cursor-pointer items-center rounded-lg border px-2.5 text-xs font-medium transition-all duration-150 active:scale-[0.97]",
                days === p
                  ? "border-primary bg-primary text-primary-foreground shadow-xs"
                  : "border-input bg-popover text-foreground hover:bg-accent/50",
              )}
            >
              {p === 365 ? "1 tahun" : `${p} hari`}
            </button>
          ))}
        </div>
        <span className="text-muted-foreground text-xs">
          Margin = omzet − HPP katalog · biaya platform belum termasuk
        </span>
      </div>

      {error && (
        <p className="text-destructive text-sm" role="alert">
          {error}
        </p>
      )}

      {/* KPI band */}
      <Card>
        <CardPanel className="grid grid-cols-2 gap-y-2 p-0! sm:grid-cols-3 lg:grid-cols-5 lg:divide-x">
          <Kpi
            label="Omzet"
            value={t ? formatIdrCompact(parseFloat(t.revenue)) : null}
            sub={t ? `${t.orders} order · AOV ${formatIdrCompact(parseFloat(t.aov))}` : undefined}
          />
          <Kpi
            label="Margin kotor"
            value={t ? formatIdrCompact(parseFloat(t.margin)) : null}
            sub={t?.marginPct != null ? `${t.marginPct}% dari omzet ter-cover` : undefined}
            tone="good"
          />
          <Kpi
            label="Coverage HPP"
            value={t ? `${Math.round(t.hppCoverage * 100)}%` : null}
            sub="qty jual yang HPP-nya ada"
          />
          <Kpi
            label="Item terjual"
            value={t ? String(t.qty) : null}
            sub={t ? `${t.items} baris item` : undefined}
          />
          <Kpi
            label="Cancel rate"
            value={t ? `${t.cancelRate}%` : null}
            sub={t ? `${t.canceledOrders} order batal` : undefined}
            tone={t && t.cancelRate >= 15 ? "bad" : undefined}
          />
        </CardPanel>
      </Card>

      {/* Daily trend */}
      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-base">Tren harian</CardTitle>
          <CardDescription>
            Omzet per hari (WIB)
            {data && data.daily.length > 0
              ? ` · ${data.daily[0].date} – ${data.daily[data.daily.length - 1].date}`
              : ""}
          </CardDescription>
        </CardHeader>
        <CardPanel>
          {loading && !data ? (
            <Skeleton className="h-36 w-full" />
          ) : (
            <div className="flex h-36 items-end gap-[3px]">
              {data?.daily.map((d) => {
                const v = parseFloat(d.revenue) || 0;
                return (
                  <div
                    key={d.date}
                    className="group flex h-full flex-1 items-end"
                    title={`${d.date} · ${formatIdr(v)} · ${d.orders} order`}
                  >
                    <div
                      className="w-full rounded-t-[2px] bg-primary/60 transition-colors group-hover:bg-primary"
                      style={{
                        height: `${Math.max(2, (v / maxRev) * 100)}%`,
                      }}
                    />
                  </div>
                );
              })}
            </div>
          )}
        </CardPanel>
      </Card>

      <div className="grid gap-4 lg:grid-cols-2">
        {/* Platform comparison */}
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-base">Perbandingan platform</CardTitle>
            <CardDescription>
              Order · omzet · margin per marketplace
            </CardDescription>
          </CardHeader>
          <CardPanel className="flex flex-col gap-3">
            {loading && !data ? (
              <>
                <Skeleton className="h-8 w-full" />
                <Skeleton className="h-8 w-full" />
              </>
            ) : (
              data?.platforms.map((p) => {
                const rev = parseFloat(p.revenue) || 0;
                const share =
                  totalPlatformRev > 0 ? (rev / totalPlatformRev) * 100 : 0;
                return (
                  <div key={p.platform} className="flex flex-col gap-1.5">
                    <div className="flex flex-wrap items-baseline gap-x-2.5 gap-y-0.5 text-sm">
                      <span className="w-16 font-medium capitalize">
                        {p.platform}
                      </span>
                      <span className="font-medium tabular-nums">
                        {formatIdrCompact(rev)}
                      </span>
                      <span className="text-muted-foreground text-xs tabular-nums">
                        {p.orders} order
                      </span>
                      <span className="text-success-foreground text-xs tabular-nums">
                        margin {formatIdrCompact(parseFloat(p.margin))}
                        {p.marginPct != null ? ` (${p.marginPct}%)` : ""}
                      </span>
                      <span className="text-muted-foreground text-xs">
                        HPP {Math.round(p.hppCoverage * 100)}% · batal{" "}
                        {p.canceledOrders}
                      </span>
                    </div>
                    <div className="h-1.5 overflow-hidden rounded-full bg-muted">
                      <div
                        className="h-full rounded-full bg-primary/70"
                        style={{ width: `${Math.max(2, share)}%` }}
                      />
                    </div>
                  </div>
                );
              })
            )}
          </CardPanel>
        </Card>

        {/* Carriers */}
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-base">Ekspedisi</CardTitle>
            <CardDescription>Jumlah order per kurir</CardDescription>
          </CardHeader>
          <CardPanel className="flex flex-col gap-1.5">
            {loading && !data ? (
              <>
                <Skeleton className="h-4 w-full" />
                <Skeleton className="h-4 w-4/5" />
                <Skeleton className="h-4 w-3/5" />
              </>
            ) : (
              data?.carriers.slice(0, 10).map((c) => (
                <div key={c.carrier} className="flex items-center gap-2.5 text-sm">
                  <span
                    className="w-28 shrink-0 truncate text-muted-foreground"
                    title={c.carrier}
                  >
                    {c.carrier}
                  </span>
                  <div className="h-1.5 min-w-0 flex-1 overflow-hidden rounded-full bg-muted">
                    <div
                      className="h-full rounded-full bg-primary/70"
                      style={{
                        width: `${Math.max(3, (c.count / maxCarrier) * 100)}%`,
                      }}
                    />
                  </div>
                  <span className="w-10 shrink-0 text-right font-medium tabular-nums">
                    {c.count}
                  </span>
                </div>
              ))
            )}
          </CardPanel>
        </Card>
      </div>

      {/* Top products */}
      <Card>
        <CardHeader className="pb-2">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div>
              <CardTitle className="text-base">Top produk</CardTitle>
              <CardDescription>
                10 teratas menurut {topTab === "revenue" ? "omzet" : "margin"}
              </CardDescription>
            </div>
            <div className="flex gap-1.5">
              {(["revenue", "margin"] as const).map((tab) => (
                <button
                  key={tab}
                  type="button"
                  onClick={() => setTopTab(tab)}
                  className={cn(
                    "inline-flex h-7 cursor-pointer items-center rounded-lg border px-2.5 text-xs font-medium transition-all duration-150 active:scale-[0.97]",
                    topTab === tab
                      ? "border-primary bg-primary text-primary-foreground shadow-xs"
                      : "border-input bg-popover text-foreground hover:bg-accent/50",
                  )}
                >
                  {tab === "revenue" ? "Omzet" : "Margin"}
                </button>
              ))}
            </div>
          </div>
        </CardHeader>
        <CardPanel className="p-0!">
          {loading && !data ? (
            <div className="flex flex-col gap-2 p-4">
              <Skeleton className="h-8 w-full" />
              <Skeleton className="h-8 w-full" />
              <Skeleton className="h-8 w-full" />
            </div>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="w-8">#</TableHead>
                  <TableHead>SKU</TableHead>
                  <TableHead>Produk</TableHead>
                  <TableHead className="text-right">Qty</TableHead>
                  <TableHead className="text-right">Omzet</TableHead>
                  <TableHead className="text-right">Margin</TableHead>
                  <TableHead className="text-right">%</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {topProducts.map((p, i) => (
                  <TableRow key={p.sku}>
                    <TableCell className="text-muted-foreground tabular-nums">
                      {i + 1}
                    </TableCell>
                    <TableCell className="font-mono text-xs">{p.sku}</TableCell>
                    <TableCell
                      className="max-w-[260px] truncate text-muted-foreground"
                      title={p.name ?? undefined}
                    >
                      {p.name ?? "—"}
                    </TableCell>
                    <TableCell className="text-right tabular-nums">
                      {p.qty}
                    </TableCell>
                    <TableCell className="text-right tabular-nums">
                      {formatIdr(parseFloat(p.revenue))}
                    </TableCell>
                    <TableCell className="text-right tabular-nums">
                      {formatIdr(parseFloat(p.margin))}
                    </TableCell>
                    <TableCell className="text-muted-foreground text-right tabular-nums">
                      {p.marginPct != null ? `${p.marginPct}%` : "—"}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </CardPanel>
      </Card>

      {/* State funnel */}
      {data && data.states.length > 0 && (
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="text-muted-foreground text-xs">State order:</span>
          {data.states.map((s) => (
            <Badge key={s.state} variant="outline" className="capitalize">
              {s.state}
              <span className="tabular-nums">{s.count}</span>
            </Badge>
          ))}
        </div>
      )}
    </div>
  );
}

function ProductsPage() {
  const [rows, setRows] = useState<CatalogProduct[]>([]);
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
      <div className="flex flex-wrap items-center justify-end gap-3">
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
