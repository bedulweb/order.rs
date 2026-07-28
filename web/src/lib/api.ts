const TOKEN_KEY = "orders_api_token";

export type BatchSession = "morning" | "afternoon" | "urgent";

export type BacklogOrder = {
  orderId: number;
  platformOrderId: string;
  platform: string;
  carrier: string | null;
  isUrgent: boolean;
  orderedAt: string | null;
  itemTotalNum: number | null;
};

export type BacklogResponse = {
  total: number;
  urgentCount: number;
  orders: BacklogOrder[];
};

export type BatchSummary = {
  id: string;
  session: string;
  status: string;
  timezone: string;
  orderCount: number;
  urgentCount: number;
  pdfFilename: string | null;
  createdAt: string;
  createdAtWib: string;
};

export type BatchLineItem = {
  sku: string | null;
  name: string | null;
  quantity: number;
};

export type BatchMember = {
  orderId: number;
  platformOrderId: string;
  platform: string | null;
  carrierSnapshot: string | null;
  isUrgent: boolean;
  position: number;
  orderedAt: string | null;
  items: BatchLineItem[];
};

export type BatchDetail = BatchSummary & {
  members: BatchMember[];
};

export type BatchesListResponse = {
  date: string;
  timezone: string;
  batches: BatchSummary[];
};

export class ApiError extends Error {
  status: number;
  body: unknown;
  constructor(status: number, message: string, body?: unknown) {
    super(message);
    this.status = status;
    this.body = body;
  }
}

export function getToken(): string | null {
  try {
    return sessionStorage.getItem(TOKEN_KEY);
  } catch {
    return null;
  }
}

export function setToken(token: string): void {
  sessionStorage.setItem(TOKEN_KEY, token.trim());
}

export function clearToken(): void {
  sessionStorage.removeItem(TOKEN_KEY);
}

async function request<T>(
  path: string,
  init: RequestInit = {},
): Promise<T> {
  const token = getToken();
  const headers = new Headers(init.headers);
  if (token) {
    headers.set("Authorization", `Bearer ${token}`);
    headers.set("X-Api-Key", token);
  }
  if (init.body && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }
  const res = await fetch(path, { ...init, headers });
  if (!res.ok) {
    let body: unknown = null;
    let message = res.statusText;
    try {
      body = await res.json();
      if (
        body &&
        typeof body === "object" &&
        "error" in body &&
        typeof (body as { error: unknown }).error === "string"
      ) {
        message = (body as { error: string }).error;
      } else if (
        body &&
        typeof body === "object" &&
        "message" in body &&
        typeof (body as { message: unknown }).message === "string"
      ) {
        message = (body as { message: string }).message;
      }
    } catch {
      /* ignore */
    }
    throw new ApiError(res.status, message, body);
  }
  if (res.status === 204) {
    return undefined as T;
  }
  const ct = res.headers.get("content-type") ?? "";
  if (ct.includes("application/json")) {
    return (await res.json()) as T;
  }
  return (await res.arrayBuffer()) as T;
}

export function fetchBacklog(limit = 500): Promise<BacklogResponse> {
  return request(`/v1/batches/backlog?limit=${limit}`);
}

// ---------------------------------------------------------------------------
// Incoming orders feed (state=new)
// ---------------------------------------------------------------------------

export type NewOrderItem = {
  sku: string | null;
  itemName: string | null;
  variantAttr: string | null;
  quantity: number;
  unitPrice: string | null;
  amount: string | null;
  imageUrl: string | null;
};

export type NewOrder = {
  orderId: number;
  platformOrderId: string;
  platform: string;
  shopName: string | null;
  buyerUsername: string | null;
  contactPerson: string | null;
  carrier: string | null;
  isUrgent: boolean;
  /** Summary List already printed (batch membership and/or BigSeller marks). */
  summaryPrinted: boolean;
  /** Shipping label (resi) already printed (BigSeller printLabelMark). */
  labelPrinted: boolean;
  /** Active batch session owning this order (morning/afternoon/urgent), if any. */
  batchSession: string | null;
  amount: string | null;
  itemTotalNum: number | null;
  orderedAt: string | null;
  items: NewOrderItem[];
};

export type FeedStatus = "new" | "processing" | "shipped" | "completed" | "all";

export type FeedCounts = {
  new: number;
  processing: number;
  shipped: number;
  completed: number;
  all: number;
  /** Today's processing orders whose resi is not printed yet. */
  unprintedLabels: number;
};

export type OrdersFeedResponse = {
  total: number;
  counts: FeedCounts;
  orders: NewOrder[];
};

/** Orders feed mirroring BigSeller's status tabs, server-paginated. */
export function fetchOrdersFeed(opts: {
  status: FeedStatus;
  q?: string;
  limit?: number;
  offset?: number;
}): Promise<OrdersFeedResponse> {
  const params = new URLSearchParams();
  params.set("status", opts.status);
  if (opts.q?.trim()) params.set("q", opts.q.trim());
  params.set("limit", String(opts.limit ?? 50));
  params.set("offset", String(opts.offset ?? 0));
  return request(`/v1/orders/new?${params.toString()}`);
}

export function fetchBatchesToday(date?: string): Promise<BatchesListResponse> {
  const q = date ? `?date=${encodeURIComponent(date)}` : "";
  return request(`/v1/batches${q}`);
}

export function createBatch(session: BatchSession): Promise<BatchDetail> {
  return request("/v1/batches", {
    method: "POST",
    body: JSON.stringify({ session }),
  });
}

export type SkippedOrder = {
  orderId: number;
  platformOrderId: string | null;
  reason: string;
};

export type SelectionBatchResult = BatchDetail & {
  skipped: SkippedOrder[];
};

/** Claim an explicit order selection into a new batch + Summary List PDF. */
export function createBatchFromSelection(
  session: BatchSession,
  orderIds: number[],
): Promise<SelectionBatchResult> {
  return request("/v1/batches/from-selection", {
    method: "POST",
    body: JSON.stringify({ session, orderIds }),
  });
}

export type PackResult = {
  requested: number;
  /** Orders BigSeller accepted for packing. */
  packed: number[];
  /** Requested but no longer packable (already moved/packed). */
  skipped: number[];
  ok: boolean;
  message: string;
};

/** Pack orders in BigSeller (mirror of the web "Pack" button). */
export function packOrders(orderIds: number[]): Promise<PackResult> {
  return request("/v1/orders/pack", {
    method: "POST",
    body: JSON.stringify({ orderIds }),
  });
}

// --- On-demand BigSeller sync (Refresh dialog) ---

export type SyncStepState = "pending" | "running" | "ok" | "error";

export interface SyncStep {
  key: string;
  label: string;
  state: SyncStepState;
  detail?: string;
}

export interface SyncRunResult {
  pages: number;
  upserted: number;
  created: number;
  stateChanged: number;
  healed: number;
  archived: number;
}

export interface SyncProgress {
  running: boolean;
  /** Epoch ms of the run start; 0 when nothing has run since API boot. */
  startedAtMs: number;
  finishedAtMs?: number;
  ok?: boolean;
  error?: string;
  steps: SyncStep[];
  result?: SyncRunResult;
}

/** Kick off an on-demand sync; `started` is false when one is in flight. */
export function startSyncRun(): Promise<{ ok: boolean; started: boolean }> {
  return request("/v1/sync/run", { method: "POST" });
}

/** Live step progress for the Refresh dialog. */
export function fetchSyncProgress(): Promise<SyncProgress> {
  return request("/v1/sync/progress");
}

// --- Today stats (ops home dashboard) ---

export interface CarrierCount {
  carrier: string;
  count: number;
}

export interface TopProduct {
  sku: string;
  name?: string;
  qty: number;
}

export interface PlatformCount {
  platform: string;
  count: number;
}

export interface TodayStats {
  /** WIB calendar date YYYY-MM-DD. */
  date: string;
  totalOrders: number;
  totalItems: number;
  platforms: PlatformCount[];
  carriers: CarrierCount[];
  topProducts: TopProduct[];
}

/** Today's (WIB) order volume per carrier + busiest products. */
export function fetchTodayStats(): Promise<TodayStats> {
  return request("/v1/stats/today");
}

// --- Analytics dashboard ---

export interface AnalyticsTotals {
  orders: number;
  canceledOrders: number;
  /** Percent (0-100). */
  cancelRate: number;
  items: number;
  qty: number;
  /** Fraction (0-1) of sold qty whose HPP is known. */
  hppCoverage: number;
  /** Decimal strings (IDR). */
  revenue: string;
  revenueCovered: string;
  cost: string;
  margin: string;
  /** Margin % over covered revenue. */
  marginPct?: number;
  aov: string;
}

export interface AnalyticsDay {
  date: string;
  orders: number;
  revenue: string;
  margin: string;
}

export interface AnalyticsPlatform {
  platform: string;
  orders: number;
  canceledOrders: number;
  items: number;
  revenue: string;
  cost: string;
  margin: string;
  marginPct?: number;
  hppCoverage: number;
}

export interface AnalyticsProduct {
  sku: string;
  name?: string;
  qty: number;
  revenue: string;
  cost: string;
  margin: string;
  marginPct?: number;
}

export interface StateCount {
  state: string;
  count: number;
}

export interface Analytics {
  days: number;
  currency: string;
  totals: AnalyticsTotals;
  daily: AnalyticsDay[];
  platforms: AnalyticsPlatform[];
  carriers: CarrierCount[];
  states: StateCount[];
  topRevenue: AnalyticsProduct[];
  topMargin: AnalyticsProduct[];
}

/** Comprehensive analytics for the last `days` days. */
export function fetchAnalytics(days: number): Promise<Analytics> {
  return request(`/v1/analytics?days=${days}`);
}

// --- Resi (shipping label) bulk print via the BigSeller print plugin ---

export interface ResiLabel {
  orderId: number;
  platformOrderId: string;
  platform: string;
  packageNo?: string;
  trackingNo?: string;
  shippingCarrierName?: string;
}

export interface ResiPrep {
  /** Encrypted puid for the plugin `setPuid` handshake. */
  encryptId: string;
  uid: string;
  bufferPrintUser: boolean;
  labels: ResiLabel[];
  /** Requested orders BigSeller refused (canceled / not printable). */
  notPrintable: number[];
  shipProvider?: string;
}

/** Validate + buffer shipping labels in BigSeller; returns plugin handshake material. */
export function prepareResiPrint(orderIds: number[]): Promise<ResiPrep> {
  return request("/v1/orders/resi-print", {
    method: "POST",
    body: JSON.stringify({ orderIds }),
  });
}

/** Register the buffered labels as a print job (confirmLabelPrint) — the
 *  connected plugin picks them up and prints. */
export function confirmResiPrint(orderIds: number[]): Promise<unknown> {
  return request("/v1/orders/resi-confirm", {
    method: "POST",
    body: JSON.stringify({ orderIds }),
  });
}

/** Internal ids of today's processing orders whose resi is not printed yet. */
export function fetchUnprintedResiIds(): Promise<{ orderIds: number[]; count: number }> {
  return request("/v1/orders/resi-unprinted");
}

/** One pick-list PDF for an explicit selection — no batch, no claim. */
export async function downloadPicklistPdf(orderIds: number[]): Promise<void> {
  const token = getToken();
  const headers = new Headers({ "Content-Type": "application/json" });
  if (token) {
    headers.set("Authorization", `Bearer ${token}`);
    headers.set("X-Api-Key", token);
  }
  const res = await fetch("/v1/orders/picklist-pdf", {
    method: "POST",
    headers,
    body: JSON.stringify({ orderIds }),
  });
  if (!res.ok) {
    let message = "Failed to generate PDF";
    try {
      const body = (await res.json()) as { error?: string };
      if (body?.error) message = body.error;
    } catch {
      /* ignore */
    }
    throw new ApiError(res.status, message);
  }
  const blob = await res.blob();
  const cd = res.headers.get("content-disposition") ?? "";
  const match = /filename="?([^";]+)"?/i.exec(cd);
  const filename = match?.[1] ?? "picklist.pdf";
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

export function fetchBatch(id: string): Promise<BatchDetail> {
  return request(`/v1/batches/${id}`);
}

/** Rebuild Summary List PDF for an existing batch (membership unchanged). */
export function regenerateBatchPdf(id: string): Promise<BatchDetail> {
  return request(`/v1/batches/${id}/regenerate-pdf`, {
    method: "POST",
  });
}

export async function downloadBatchPdf(id: string, filenameHint?: string): Promise<void> {
  const token = getToken();
  const headers = new Headers();
  if (token) {
    headers.set("Authorization", `Bearer ${token}`);
    headers.set("X-Api-Key", token);
  }
  const res = await fetch(`/v1/batches/${id}/pdf`, { headers });
  if (!res.ok) {
    throw new ApiError(res.status, "Failed to download PDF");
  }
  const blob = await res.blob();
  const cd = res.headers.get("content-disposition") ?? "";
  const match = /filename="?([^";]+)"?/i.exec(cd);
  const filename = match?.[1] ?? filenameHint ?? `batch-${id}.pdf`;
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

export function formatWib(iso: string | null | undefined): string {
  if (!iso) return "—";
  try {
    const d = new Date(iso);
    return new Intl.DateTimeFormat("en-GB", {
      timeZone: "Asia/Jakarta",
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      hour12: false,
    }).format(d) + " WIB";
  } catch {
    return iso;
  }
}

// ---------------------------------------------------------------------------
// Product catalog (ART + HPP)
// ---------------------------------------------------------------------------

export type CatalogProduct = {
  art: string;
  name: string;
  hpp: number;
};

export type CatalogListResponse = {
  total: number;
  products: CatalogProduct[];
};

export type CatalogImportResult = {
  inserted: number;
  updated: number;
  skipped: number;
  totalRows: number;
};

export function fetchCatalogProducts(opts?: {
  q?: string;
  limit?: number;
  offset?: number;
}): Promise<CatalogListResponse> {
  const params = new URLSearchParams();
  if (opts?.q?.trim()) params.set("q", opts.q.trim());
  if (opts?.limit != null) params.set("limit", String(opts.limit));
  if (opts?.offset != null) params.set("offset", String(opts.offset));
  const qs = params.toString();
  return request(`/v1/catalog/products${qs ? `?${qs}` : ""}`);
}

export function fetchCatalogProduct(art: string): Promise<CatalogProduct> {
  return request(`/v1/catalog/products/${encodeURIComponent(art)}`);
}

/** Import from server default workbook path (or optional path). */
export function importCatalog(path?: string): Promise<CatalogImportResult> {
  return request("/v1/catalog/import", {
    method: "POST",
    body: JSON.stringify(path ? { path } : {}),
  });
}

export function formatIdr(n: number | null | undefined): string {
  if (n == null || Number.isNaN(n)) return "—";
  return new Intl.NumberFormat("id-ID", {
    style: "currency",
    currency: "IDR",
    maximumFractionDigits: 0,
  }).format(n);
}
