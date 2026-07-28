import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
  flexRender,
  getCoreRowModel,
  useReactTable,
  type ColumnDef,
} from "@tanstack/react-table";
import {
  BadgeCheck,
  ChevronLeft,
  ChevronRight,
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
  createBatchFromSelection,
  downloadBatchPdf,
  downloadPicklistPdf,
  fetchBacklog,
  fetchBatch,
  fetchBatchesToday,
  regenerateBatchPdf,
  fetchCatalogProducts,
  fetchOrdersFeed,
  formatIdr,
  formatWib,
  getToken,
  importCatalog,
  packOrders,
  setToken,
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
  type SelectionBatchResult,
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
            <NavBtn to="/order-masuk">Order Masuk</NavBtn>
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
          <Route path="/order-masuk" element={<NewOrdersPage />} />
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
  const [refreshing, setRefreshing] = useState(false);
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
      setRefreshing(true);
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
        setRefreshing(false);
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
  // --- + carrier (BigSeller only bulk-prints within one carrier).
  const [resiOpen, setResiOpen] = useState(false);
  const [copiedKey, setCopiedKey] = useState<string | null>(null);
  const [pdfBusy, setPdfBusy] = useState(false);

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
      <div>
        <Button size="sm" variant="ghost" render={<Link to="/" />}>
          ← Home
        </Button>
        <h1 className="font-heading text-2xl font-semibold tracking-tight">
          Order masuk
        </h1>
        <p className="text-muted-foreground text-sm">
          Cermin tab order BigSeller · terbaru di atas
          {appliedQ ? ` · cari: “${appliedQ}”` : ""}
        </p>
      </div>

      <div className="flex flex-wrap items-center gap-2">
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

        <div className="relative ms-auto w-full sm:w-72">
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
      </div>

      <div className="flex flex-wrap items-center justify-end gap-2">
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
            loading={refreshing}
            onClick={() => void load(true)}
          >
            Refresh
          </Button>
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
              <Button
                size="sm"
                onClick={() => {
                  setCopiedKey(null);
                  setResiOpen(true);
                }}
                title="Kelompokkan per marketplace + ekspedisi untuk bulk print di BigSeller"
              >
                <Printer className="size-3.5" /> Print resi (
                {selectedOrders.size})
              </Button>
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
    </div>
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
