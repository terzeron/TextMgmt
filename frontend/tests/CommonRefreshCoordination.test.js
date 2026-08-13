// @vitest-environment jsdom
//
// Phase 1: 크로스탭 refresh single-flight 검증.
//
// 서버는 refresh token 을 회전시키므로 여러 탭이 같은 토큰으로 동시에 /auth/refresh 를
// 호출하면 나중 요청이 이미 회전된 토큰을 제출하고, 서버가 이를 재사용 공격으로 오판해
// 세션 전체가 풀린다. "탭"은 모듈 인스턴스로 시뮬레이션한다(vi.resetModules + dynamic import).
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

const REFRESH_URL = "/api/auth/refresh";

function deferred() {
  let resolve;
  const promise = new Promise((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

function okResponse(expiresIn = 7200) {
  return {
    ok: true,
    status: 200,
    json: async () => ({ status: "success", expires_in: expiresIn }),
  };
}

// 여러 모듈 인스턴스(=탭)가 공유하는 Web Locks 대역. ifAvailable 의미를 그대로 흉내낸다.
function installWebLocksMock() {
  const held = new Set();
  vi.stubGlobal("navigator", {
    locks: {
      request: async (name, options, callback) => {
        if (options?.ifAvailable && held.has(name)) return callback(null);
        held.add(name);
        try {
          return await callback({ name });
        } finally {
          held.delete(name);
        }
      },
    },
  });
  return held;
}

// 각 탭은 독립된 모듈 인스턴스를 갖는다(모듈 스코프 refreshPromise 가 탭별로 분리됨).
async function openTab() {
  vi.resetModules();
  return await import("../src/Common");
}

describe("크로스탭 refresh single-flight", () => {
  let fetchMock;

  beforeEach(() => {
    fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    window.__ENV__ = { VITE_API_URL_PREFIX: "/api" };
    localStorage.clear();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.useRealTimers();
    localStorage.clear();
  });

  it("한 탭 안의 동시 호출은 하나의 요청만 보낸다", async () => {
    installWebLocksMock();
    fetchMock.mockResolvedValue(okResponse());
    const tab = await openTab();

    const results = await Promise.all([
      tab.tryRefreshToken(),
      tab.tryRefreshToken(),
      tab.tryRefreshToken(),
    ]);

    expect(results).toEqual([true, true, true]);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    tab.stopProactiveRefresh();
  });

  it("다른 탭이 회전 중이면 자체 요청을 보내지 않고 결과를 기다린다", async () => {
    installWebLocksMock();
    const pending = deferred();
    fetchMock.mockReturnValueOnce(pending.promise);

    const tabA = await openTab();
    const tabB = await openTab();

    // 탭 A 가 락을 잡고 회전을 시작한다
    const resultA = tabA.tryRefreshToken();
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));

    // 탭 B 는 락을 못 잡으므로 요청을 보내지 않고 A 의 결과를 기다린다
    const resultB = tabB.tryRefreshToken();
    pending.resolve(okResponse());

    expect(await resultA).toBe(true);
    expect(await resultB).toBe(true);
    // 핵심: 두 탭이 refresh 를 원했지만 서버로 나간 요청은 하나뿐이다
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock).toHaveBeenCalledWith(
      REFRESH_URL,
      expect.objectContaining({ method: "POST" }),
    );
    tabA.stopProactiveRefresh();
    tabB.stopProactiveRefresh();
  });

  it("성공 결과를 브로드캐스트해 대기 탭이 선제 갱신 타이머를 다시 잡는다", async () => {
    installWebLocksMock();
    vi.useFakeTimers();
    const pending = deferred();
    fetchMock.mockReturnValueOnce(pending.promise);

    const tabA = await openTab();
    const tabB = await openTab();

    const resultA = tabA.tryRefreshToken();
    await vi.advanceTimersByTimeAsync(0); // 탭 A 가 락을 잡고 요청을 보낸다
    expect(fetchMock).toHaveBeenCalledTimes(1);

    const resultB = tabB.tryRefreshToken();
    await vi.advanceTimersByTimeAsync(0); // 탭 B 는 락을 못 잡고 대기에 들어간다

    pending.resolve(okResponse(600));
    expect(await resultA).toBe(true);
    expect(await resultB).toBe(true);

    // 탭 A 의 타이머를 끄고, 탭 B 가 브로드캐스트받은 expires_in(600s) 으로 자체
    // 타이머를 예약했는지만 본다. 만료 5분 전 = 300초 후에 발동해야 한다.
    tabA.stopProactiveRefresh();
    fetchMock.mockResolvedValue(okResponse(600));
    await vi.advanceTimersByTimeAsync(299_000);
    expect(fetchMock).toHaveBeenCalledTimes(1); // 아직 발동 전
    await vi.advanceTimersByTimeAsync(2_000);
    expect(fetchMock).toHaveBeenCalledTimes(2);

    tabB.stopProactiveRefresh();
  });

  it("락 홀더가 실패해 결과가 오지 않으면 대기 탭이 직접 재시도한다", async () => {
    const held = installWebLocksMock();
    const tabB = await openTab();

    // 다른 탭이 락을 점유한 상태를 만든다
    held.add("tm-auth-refresh");
    vi.useFakeTimers();
    fetchMock.mockResolvedValue(okResponse());

    const resultB = tabB.tryRefreshToken();
    // 대기 중에는 요청을 보내지 않는다
    expect(fetchMock).not.toHaveBeenCalled();

    // 홀더가 브로드캐스트 없이 죽었다 -> 대기 타임아웃 후 직접 시도
    held.delete("tm-auth-refresh");
    await vi.advanceTimersByTimeAsync(10_000);

    expect(await resultB).toBe(true);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    tabB.stopProactiveRefresh();
  });

  it("Web Locks 미지원 브라우저에서는 localStorage 락으로 폴백한다", async () => {
    vi.stubGlobal("navigator", {});
    const pending = deferred();
    fetchMock.mockReturnValueOnce(pending.promise);

    const tabA = await openTab();
    const tabB = await openTab();

    const resultA = tabA.tryRefreshToken();
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    // 폴백 락이 점유 표시를 남긴다
    expect(localStorage.getItem("tm_auth_refresh_lock")).toBeTruthy();

    const resultB = tabB.tryRefreshToken();
    pending.resolve(okResponse());

    expect(await resultA).toBe(true);
    expect(await resultB).toBe(true);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    // 락은 해제되어야 한다
    expect(localStorage.getItem("tm_auth_refresh_lock")).toBeNull();

    tabA.stopProactiveRefresh();
    tabB.stopProactiveRefresh();
  });

  it("만료된 폴백 락은 회수한다", async () => {
    vi.stubGlobal("navigator", {});
    fetchMock.mockResolvedValue(okResponse());
    // TTL(10s) 을 넘긴 오래된 락 -> 홀더가 죽은 것으로 보고 회수
    localStorage.setItem("tm_auth_refresh_lock", String(Date.now() - 30_000));

    const tab = await openTab();
    expect(await tab.tryRefreshToken()).toBe(true);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    tab.stopProactiveRefresh();
  });

  it("BroadcastChannel이 없으면 저장소 결과만 남기고 진행한다", async () => {
    installWebLocksMock();
    vi.stubGlobal("BroadcastChannel", undefined);
    fetchMock.mockResolvedValue(okResponse());
    const tab = await openTab();

    expect(await tab.tryRefreshToken()).toBe(true);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    tab.stopProactiveRefresh();
  });

  it("BroadcastChannel 생성 실패를 무시하고 refresh를 완료한다", async () => {
    installWebLocksMock();
    vi.stubGlobal(
      "BroadcastChannel",
      class {
        constructor() {
          throw new Error("channel disabled");
        }
      },
    );
    fetchMock.mockResolvedValue(okResponse());
    const tab = await openTab();

    expect(await tab.tryRefreshToken()).toBe(true);
    tab.stopProactiveRefresh();
  });

  it("fallback lock 저장소 접근 실패 시 조율 없이 refresh를 진행한다", async () => {
    vi.stubGlobal("navigator", {});
    const getItemSpy = vi
      .spyOn(Storage.prototype, "getItem")
      .mockImplementation(() => {
        throw new Error("storage blocked");
      });
    fetchMock.mockResolvedValue(okResponse());
    const tab = await openTab();

    expect(await tab.tryRefreshToken()).toBe(true);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    getItemSpy.mockRestore();
    tab.stopProactiveRefresh();
  });

  it("Web Locks 요청이 실패하면 락 없이 refresh를 시도한다", async () => {
    vi.stubGlobal("navigator", {
      locks: {
        request: vi.fn(async () => {
          throw new Error("locks unavailable");
        }),
      },
    });
    fetchMock.mockResolvedValue(okResponse());
    const tab = await openTab();

    expect(await tab.tryRefreshToken()).toBe(true);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    tab.stopProactiveRefresh();
  });

  it("대기 후에도 락을 얻지 못하면 false를 반환한다", async () => {
    const held = installWebLocksMock();
    vi.useFakeTimers();
    vi.stubGlobal("BroadcastChannel", undefined);
    localStorage.setItem("tm_auth_refresh_result", "{bad json");
    held.add("tm-auth-refresh");
    const tab = await openTab();

    const result = tab.tryRefreshToken();
    await vi.advanceTimersByTimeAsync(10_000);

    expect(await result).toBe(false);
    expect(fetchMock).not.toHaveBeenCalled();
    tab.stopProactiveRefresh();
  });
});

describe("refreshOnVisible 디바운스", () => {
  let fetchMock;

  beforeEach(() => {
    fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    window.__ENV__ = { VITE_API_URL_PREFIX: "/api" };
    localStorage.clear();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.useRealTimers();
    localStorage.clear();
  });

  it("짧은 창 안의 반복 visibilitychange 는 한 번만 refresh 한다", async () => {
    installWebLocksMock();
    fetchMock.mockResolvedValue(okResponse());
    const tab = await openTab();

    expect(await tab.refreshOnVisible()).toBe(true);
    // 디바운스 창(3s) 안의 후속 호출은 요청을 만들지 않는다
    expect(await tab.refreshOnVisible()).toBe(false);
    expect(await tab.refreshOnVisible()).toBe(false);

    expect(fetchMock).toHaveBeenCalledTimes(1);
    tab.stopProactiveRefresh();
  });

  it("디바운스 창이 지나면 다시 refresh 한다", async () => {
    installWebLocksMock();
    fetchMock.mockResolvedValue(okResponse());
    const tab = await openTab();

    expect(await tab.refreshOnVisible()).toBe(true);

    const realNow = Date.now;
    try {
      vi.spyOn(Date, "now").mockReturnValue(realNow() + 5_000);
      expect(await tab.refreshOnVisible()).toBe(true);
    } finally {
      Date.now = realNow;
    }

    expect(fetchMock).toHaveBeenCalledTimes(2);
    tab.stopProactiveRefresh();
  });
});

describe("recordBookView", () => {
  let fetchMock;

  beforeEach(() => {
    fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    window.__ENV__ = { VITE_API_URL_PREFIX: "/api" };
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("책은 /books/view-history/{id} 로 POST 한다", async () => {
    fetchMock.mockResolvedValue({ ok: true });
    const tab = await openTab();

    expect(await tab.recordBookView("", 42)).toBe(true);
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/books/view-history/42",
      expect.objectContaining({ method: "POST", credentials: "include" }),
    );
  });

  it("만화는 prefix 를 붙여 /comics/view-history/{id} 로 POST 한다", async () => {
    fetchMock.mockResolvedValue({ ok: true });
    const tab = await openTab();

    expect(await tab.recordBookView("/comics", 7)).toBe(true);
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/comics/view-history/7",
      expect.objectContaining({ method: "POST" }),
    );
  });

  it("bookId 가 없으면 요청하지 않는다", async () => {
    const tab = await openTab();

    expect(await tab.recordBookView("", undefined)).toBe(false);
    expect(await tab.recordBookView("", 0)).toBe(false);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("서버가 거부해도 예외를 던지지 않는다", async () => {
    fetchMock.mockResolvedValue({ ok: false, status: 404 });
    const tab = await openTab();

    expect(await tab.recordBookView("", 42)).toBe(false);
  });

  it("네트워크 오류를 삼켜 열람을 막지 않는다", async () => {
    fetchMock.mockRejectedValue(new Error("offline"));
    const tab = await openTab();

    expect(await tab.recordBookView("", 42)).toBe(false);
  });
});
