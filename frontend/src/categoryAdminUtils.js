function normalizeCount(value) {
  if (typeof value !== "number" || !Number.isFinite(value)) return null;
  return Math.max(0, Math.trunc(value));
}

export function getReloadRemainingCount(status, fallback = null) {
  if (!status) return fallback ?? 0;

  const explicitRemaining = normalizeCount(status.remaining_count);
  if (explicitRemaining !== null) return explicitRemaining;

  if (status.status === "running") {
    const before = normalizeCount(status.before_count);
    if (before !== null && before > 0) {
      const indexed = normalizeCount(status.indexed_count) ?? 0;
      const deleted = normalizeCount(status.deleted_count) ?? 0;
      return Math.max(0, before - indexed - deleted);
    }
    return fallback ?? 0;
  }

  return normalizeCount(status.after_count) ?? fallback ?? 0;
}

export function formatErrorMessage(err, fallback = "오류가 발생했습니다.") {
  if (!err) return fallback;
  if (typeof err === "string") return err;
  if (err instanceof Error && err.message) return err.message;
  if (typeof err === "object") {
    if (typeof err.message === "string" && err.message) return err.message;
    if (typeof err.detail === "string" && err.detail) return err.detail;
    if (typeof err.error === "string" && err.error) return err.error;
  }
  return String(err);
}
