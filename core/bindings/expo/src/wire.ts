import type {
  DataMetadata,
  DomainPage,
  DomainSummary,
  FleetActivity,
  FleetMachine,
  FleetSnapshot,
  FleetSourceReport,
  JobCancellation,
  JobDetails,
  JobEstimate,
  JobListItem,
  JobPage,
  JobReceipt,
  JobRecord,
  JobTask,
  JobTaskSummary,
  Portal,
  PortalDomain,
  PortalDomainPage,
  PortalPage,
  PortalPose,
} from "./AukiSdkExpo.types";

type Wire = Record<string, unknown>;

function record(value: unknown): Wire {
  return value && typeof value === "object" ? value as Wire : {};
}

function text(value: unknown, fallback = ""): string {
  return typeof value === "string" ? value : fallback;
}

function nullableText(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

function num(value: unknown): number {
  return typeof value === "number" ? value : 0;
}

function field(source: Wire, snake: string, camel: string): unknown {
  if (source[snake] !== undefined) return source[snake];
  return source[camel];
}

export function mapDomainSummary(value: unknown): DomainSummary {
  const source = record(value);
  return {
    id: text(source.id),
    name: text(source.name),
    organizationId: nullableText(field(source, "organization_id", "organizationId")),
  };
}

export function mapDomainPage(value: unknown): DomainPage {
  const source = record(value);
  const domains = Array.isArray(source.domains) ? source.domains.map(mapDomainSummary) : [];
  return {
    domains,
    total: num(source.total),
    limit: num(source.limit),
    offset: num(source.offset),
  };
}

export function mapPortalDomain(value: unknown): PortalDomain {
  const source = record(value);
  return {
    ...mapDomainSummary(source),
    isDefault: Boolean(field(source, "is_default", "isDefault")),
    addedToDomainAt: text(field(source, "added_to_domain_at", "addedToDomainAt")),
  };
}

export function mapPortal(value: unknown): Portal {
  const source = record(value);
  return {
    id: text(source.id),
    shortId: text(field(source, "short_id", "shortId")),
    name: text(source.name),
    size: num(source.size),
    organizationId: nullableText(field(source, "organization_id", "organizationId")),
    defaultDomainId: nullableText(field(source, "default_domain_id", "defaultDomainId")),
    redirectUrl: nullableText(field(source, "redirect_url", "redirectUrl")),
    createdAt: text(field(source, "created_at", "createdAt")),
    updatedAt: text(field(source, "updated_at", "updatedAt")),
  };
}

export function mapPortalPage(value: unknown): PortalPage {
  const source = record(value);
  const items = Array.isArray(source.items) ? source.items.map(mapPortal) : [];
  return {
    items,
    nextCursor: nullableText(field(source, "next_cursor", "nextCursor")),
    paginated: Boolean(source.paginated),
  };
}

export function mapPortalDomainPage(value: unknown): PortalDomainPage {
  const source = record(value);
  const items = Array.isArray(source.items) ? source.items.map(mapPortalDomain) : [];
  return {
    items,
    nextCursor: nullableText(field(source, "next_cursor", "nextCursor")),
    paginated: Boolean(source.paginated),
  };
}

export function mapPortalPose(value: unknown): PortalPose {
  const source = record(value);
  return {
    id: text(source.id),
    shortId: text(field(source, "short_id", "shortId")),
    domainId: text(field(source, "domain_id", "domainId")),
    reportedSize: num(field(source, "reported_size", "reportedSize")),
    px: num(source.px),
    py: num(source.py),
    pz: num(source.pz),
    rx: num(source.rx),
    ry: num(source.ry),
    rz: num(source.rz),
    rw: num(source.rw),
    latitude: typeof source.latitude === "number" ? source.latitude : null,
    longitude: typeof source.longitude === "number" ? source.longitude : null,
    altitude: typeof source.altitude === "number" ? source.altitude : null,
    verticalAccuracy: typeof field(source, "vertical_accuracy", "verticalAccuracy") === "number"
      ? field(source, "vertical_accuracy", "verticalAccuracy") as number
      : null,
    horizontalAccuracy: typeof field(source, "horizontal_accuracy", "horizontalAccuracy") === "number"
      ? field(source, "horizontal_accuracy", "horizontalAccuracy") as number
      : null,
    gpsTimestamp: typeof field(source, "gps_timestamp", "gpsTimestamp") === "number"
      ? field(source, "gps_timestamp", "gpsTimestamp") as number
      : null,
    scannerDeviceId: text(field(source, "scanner_device_id", "scannerDeviceId")),
    scannerDeviceName: text(field(source, "scanner_device_name", "scannerDeviceName")),
    scannerDeviceModel: text(field(source, "scanner_device_model", "scannerDeviceModel")),
    placedAt: text(field(source, "placed_at", "placedAt")),
  };
}

export function mapDataMetadata(value: unknown): DataMetadata {
  const source = record(value);
  return {
    id: text(source.id),
    domainId: text(field(source, "domain_id", "domainId")),
    name: text(source.name),
    dataType: text(field(source, "data_type", "dataType")),
    size: num(source.size),
    createdAt: text(field(source, "created_at", "createdAt")),
    updatedAt: text(field(source, "updated_at", "updatedAt")),
  };
}

function mapTaskSummary(value: unknown): JobTaskSummary {
  const source = record(value);
  return {
    queued: num(source.queued),
    leased: num(source.leased),
    running: num(source.running),
    completed: num(source.completed),
    failed: num(source.failed),
    canceled: num(source.canceled),
  };
}

function mapJobRecord(value: unknown): JobRecord {
  const source = record(value);
  return {
    id: text(source.id),
    label: text(source.label),
    domainId: text(field(source, "domain_id", "domainId")),
    status: source.status as JobRecord["status"],
    priority: num(source.priority),
    createdAt: text(field(source, "created_at", "createdAt")),
    updatedAt: text(field(source, "updated_at", "updatedAt")),
    organizationId: nullableText(field(source, "organization_id", "organizationId")),
    meta: (source.meta ?? null) as JobRecord["meta"],
    creditLockId: nullableText(field(source, "credit_lock_id", "creditLockId")),
    creditLockAmount: nullableText(field(source, "credit_lock_amount", "creditLockAmount")),
    creditLockedAt: nullableText(field(source, "credit_locked_at", "creditLockedAt")),
    creditReleasedAt: nullableText(field(source, "credit_released_at", "creditReleasedAt")),
  };
}

function mapJobTask(value: unknown): JobTask {
  const source = record(value);
  const filters = field(source, "capability_filters", "capabilityFilters");
  return {
    id: text(source.id),
    jobId: text(field(source, "job_id", "jobId")),
    label: text(source.label),
    stage: text(source.stage),
    capability: text(source.capability),
    capabilityFilters: filters && typeof filters === "object" ? filters as Record<string, string> : {},
    status: source.status as JobTask["status"],
    depsRemaining: num(field(source, "deps_remaining", "depsRemaining")),
    priority: num(source.priority),
    inputsCids: Array.isArray(field(source, "inputs_cids", "inputsCids"))
      ? field(source, "inputs_cids", "inputsCids") as string[]
      : [],
    outputsPrefix: nullableText(field(source, "outputs_prefix", "outputsPrefix")),
    organizationId: nullableText(field(source, "organization_id", "organizationId")),
    attempts: num(source.attempts),
    maxAttempts: num(field(source, "max_attempts", "maxAttempts")),
    leaseExpiresAt: nullableText(field(source, "lease_expires_at", "leaseExpiresAt")),
    reservedBy: nullableText(field(source, "reserved_by", "reservedBy")),
    meta: (source.meta ?? null) as JobTask["meta"],
    cancelRequestedAt: nullableText(field(source, "cancel_requested_at", "cancelRequestedAt")),
    lastHeartbeatAt: nullableText(field(source, "last_heartbeat_at", "lastHeartbeatAt")),
    createdAt: text(field(source, "created_at", "createdAt")),
    updatedAt: text(field(source, "updated_at", "updatedAt")),
    mode: source.mode as JobTask["mode"],
    billingUnits: text(field(source, "billing_units", "billingUnits")),
    estimatedCreditCost: nullableText(field(source, "estimated_credit_cost", "estimatedCreditCost")),
    debitedAmount: nullableText(field(source, "debited_amount", "debitedAmount")),
    debitedAt: nullableText(field(source, "debited_at", "debitedAt")),
  };
}

function mapReceipt(value: unknown): JobReceipt {
  const source = record(value);
  return {
    id: text(source.id),
    jobId: text(field(source, "job_id", "jobId")),
    taskId: text(field(source, "task_id", "taskId")),
    nodeId: nullableText(field(source, "node_id", "nodeId")),
    outputs: Array.isArray(source.outputs) ? source.outputs as string[] : [],
    meta: (source.meta ?? null) as JobReceipt["meta"],
    createdAt: text(field(source, "created_at", "createdAt")),
  };
}

export function mapJobEstimate(value: unknown): JobEstimate {
  const source = record(value);
  const tasks = Array.isArray(source.tasks) ? source.tasks : [];
  return {
    total: text(source.total),
    tasks: tasks.map(task => {
      const row = record(task);
      return {
        label: text(row.label),
        stage: text(row.stage),
        capability: text(row.capability),
        mode: row.mode as JobEstimate["tasks"][number]["mode"],
        billingUnits: text(field(row, "billing_units", "billingUnits")),
        estimatedCreditCost: text(field(row, "estimated_credit_cost", "estimatedCreditCost")),
      };
    }),
  };
}

export function mapJobPage(value: unknown): JobPage {
  const source = record(value);
  const items = Array.isArray(source.items) ? source.items : [];
  return {
    items: items.map(item => {
      const row = record(item);
      return {
        job: mapJobRecord(row.job),
        tasksSummary: mapTaskSummary(field(row, "tasks_summary", "tasksSummary")),
      } satisfies JobListItem;
    }),
    nextCursor: nullableText(field(source, "next_cursor", "nextCursor")),
  };
}

export function mapJobDetails(value: unknown): JobDetails {
  const source = record(value);
  return {
    job: mapJobRecord(source.job),
    tasksSummary: mapTaskSummary(field(source, "tasks_summary", "tasksSummary")),
    tasks: Array.isArray(source.tasks) ? source.tasks.map(mapJobTask) : [],
    receipts: Array.isArray(source.receipts) ? source.receipts.map(mapReceipt) : [],
  };
}

export function mapJobCancellation(value: unknown): JobCancellation {
  const source = record(value);
  return {
    id: text(source.id),
    status: source.status as JobCancellation["status"],
    updatedAt: text(field(source, "updated_at", "updatedAt")),
  };
}

function mapFleetActivity(value: unknown): FleetActivity {
  const source = record(value);
  return {
    workerId: text(field(source, "worker_id", "workerId")),
    jobId: text(field(source, "job_id", "jobId")),
    taskId: text(field(source, "task_id", "taskId")),
    taskStatus: field(source, "task_status", "taskStatus") as FleetActivity["taskStatus"],
    jobStatus: field(source, "job_status", "jobStatus") as FleetActivity["jobStatus"],
    mode: source.mode as FleetActivity["mode"],
    capability: text(source.capability),
    leaseExpiresAt: nullableText(field(source, "lease_expires_at", "leaseExpiresAt")),
    lastHeartbeatAt: nullableText(field(source, "last_heartbeat_at", "lastHeartbeatAt")),
    updatedAt: text(field(source, "updated_at", "updatedAt")),
    observedAt: text(field(source, "observed_at", "observedAt")),
  };
}

function mapFleetMachine(value: unknown): FleetMachine {
  const source = record(value);
  const activity = field(source, "activity", "activity");
  return {
    kind: source.kind as FleetMachine["kind"],
    id: text(source.id),
    organizationId: text(field(source, "organization_id", "organizationId")),
    name: text(source.name),
    capabilities: Array.isArray(source.capabilities) ? source.capabilities as string[] : [],
    mode: text(source.mode),
    association: source.association as FleetMachine["association"],
    presence: source.presence as FleetMachine["presence"],
    providerStatus: text(field(source, "provider_status", "providerStatus")),
    presenceObservedAt: text(field(source, "presence_observed_at", "presenceObservedAt")),
    lastSeenAt: nullableText(field(source, "last_seen_at", "lastSeenAt")),
    activeLeaseExpiresAt: nullableText(field(source, "active_lease_expires_at", "activeLeaseExpiresAt")),
    workState: field(source, "work_state", "workState") as FleetMachine["workState"],
    workObservedAt: nullableText(field(source, "work_observed_at", "workObservedAt")),
    activity: Array.isArray(activity) ? activity.map(mapFleetActivity) : [],
  };
}

function mapFleetSource(value: unknown): FleetSourceReport {
  const source = record(value);
  const status = field(source, "http_status", "httpStatus");
  return {
    source: source.source as FleetSourceReport["source"],
    state: source.state as FleetSourceReport["state"],
    observedAt: text(field(source, "observed_at", "observedAt")),
    codes: Array.isArray(source.codes) ? source.codes as string[] : [],
    httpStatus: typeof status === "number" ? status : null,
  };
}

export function mapFleetSnapshot(value: unknown): FleetSnapshot {
  const source = record(value);
  const unresolved = field(source, "unresolved_activity", "unresolvedActivity");
  return {
    domainId: text(field(source, "domain_id", "domainId")),
    view: source.view as FleetSnapshot["view"],
    observedAt: text(field(source, "observed_at", "observedAt")),
    machines: Array.isArray(source.machines) ? source.machines.map(mapFleetMachine) : [],
    unresolvedActivity: Array.isArray(unresolved) ? unresolved.map(mapFleetActivity) : [],
    sources: Array.isArray(source.sources) ? source.sources.map(mapFleetSource) : [],
    complete: Boolean(source.complete),
  };
}
