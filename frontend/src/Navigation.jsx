import { useEffect, useState, useCallback } from "react";
import { Outlet, useLocation, useNavigate, Navigate } from "react-router-dom";
import {
  GoogleOAuthProvider,
  GoogleLogin,
  googleLogout,
} from "@react-oauth/google";
import {
  Button,
  Form,
  FormControl,
  InputGroup,
  Nav,
  Navbar,
  Dropdown,
} from "react-bootstrap";
import { FontAwesomeIcon } from "@fortawesome/react-fontawesome";
import { faSearch, faSpinner, faUser } from "@fortawesome/free-solid-svg-icons";
import {
  rawJsonGetReq,
  getApiUrlPrefix,
  tryRefreshToken,
  refreshOnVisible,
  startProactiveRefresh,
  stopProactiveRefresh,
} from "./Common.js";
import { isViewerAllowedPath } from "./auth.js";

export default function Navigation() {
  /* v8 ignore next 3 -- Vite env fallback depends on the runtime bundle. */
  const clientId =
    window.__ENV__?.["VITE_GOOGLE_CLIENT_ID"] ||
    import.meta.env.VITE_GOOGLE_CLIENT_ID;

  const [sessionLoading, setSessionLoading] = useState(true);
  const [login, setLogin] = useState(false);
  const [role, setRole] = useState(null); // 'admin' | 'viewer' | null
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [picture, setPicture] = useState("");
  const [searchKeyword, setSearchKeyword] = useState("");
  const [searchResults, setSearchResults] = useState([]);
  const [hasSearched, setHasSearched] = useState(false);
  const [searchTotal, setSearchTotal] = useState(0);
  const [searchLoading, setSearchLoading] = useState(false);
  const [searchInProgress, setSearchInProgress] = useState(false);
  const [hiddenCategories, setHiddenCategories] = useState([]);

  const location = useLocation();
  const navigate = useNavigate();
  const isComicsContext = location.pathname.startsWith("/comics-");
  const searchPrefix = isComicsContext ? "/comics" : "";

  // viewer일 때 비노출 카테고리 목록 로드
  useEffect(() => {
    if (role === "viewer") {
      const contentType = isComicsContext ? "comic" : "book";
      setHiddenCategories([]); // 컨텍스트 전환 시 stale 데이터 방지
      rawJsonGetReq(
        `/hidden-categories?content_type=${contentType}`,
        (data) => {
          if (data.status === "success") {
            setHiddenCategories(data.result || []);
          }
        },
        () => {
          setHiddenCategories([]);
        },
      );
    } else {
      setHiddenCategories([]);
    }
  }, [role, isComicsContext]);

  const buildSearchUrl = (keyword, offset, limit) => {
    let url = `${searchPrefix}/search/${encodeURIComponent(keyword)}?offset=${offset}&limit=${limit}`;
    if (hiddenCategories.length > 0) {
      url += `&exclude_categories=${encodeURIComponent(hiddenCategories.join(","))}`;
    }
    return url;
  };

  // 검색 실행 로직을 함수로 추출
  const handleSearch = () => {
    if (searchKeyword) {
      // 홈 화면에서 검색 시 책 조회 페이지로 이동
      if (location.pathname === "/") {
        navigate("/book-view");
      }
      setHasSearched(true);
      setSearchInProgress(true);
      rawJsonGetReq(
        buildSearchUrl(searchKeyword, 0, 10),
        (data) => {
          if (data.status === "success") {
            setSearchResults(data.result || []);
            setSearchTotal(data.total || 0);
          } else {
            setSearchResults([]);
            setSearchTotal(0);
          }
          setSearchInProgress(false);
        },
        (error) => {
          console.error(error);
          setSearchResults([]);
          setSearchTotal(0);
          setSearchInProgress(false);
        },
      );
    }
  };

  const handleLoadMore = useCallback(() => {
    if (searchLoading || !searchKeyword) return;
    setSearchLoading(true);
    const offset = searchResults.length;
    rawJsonGetReq(
      buildSearchUrl(searchKeyword, offset, 10),
      (data) => {
        if (data.status === "success" && data.result) {
          setSearchResults((prev) => [...prev, ...data.result]);
          setSearchTotal(data.total || 0);
        }
        setSearchLoading(false);
      },
      (error) => {
        console.error(error);
        setSearchLoading(false);
      },
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps -- buildSearchUrl은 안정적 helper로 deps에 포함 시 매 렌더마다 재실행 유발 (의도된 누락)
  }, [
    searchKeyword,
    searchResults.length,
    searchLoading,
    hiddenCategories,
    searchPrefix,
  ]);

  useEffect(() => {
    if (!clientId) {
      console.error(
        "The environment variable VITE_GOOGLE_CLIENT_ID is not set.",
      );
      setSessionLoading(false);
      return;
    }
    const loadSession = async () => {
      try {
        let res = await fetch(getApiUrlPrefix() + "/auth/me", {
          credentials: "include",
        });
        if (res.status === 401) {
          const refreshed = await tryRefreshToken();
          if (!refreshed) return;
          res = await fetch(getApiUrlPrefix() + "/auth/me", {
            credentials: "include",
          });
        }
        if (!res.ok) return;
        const data = await res.json();
        if (data.status === "success" && data.result?.role) {
          setLogin(true);
          setRole(data.result.role);
          setName(data.result.name || "");
          setEmail(data.result.email || "");
          setPicture(data.result.picture || "");
          // 세션 복원 성공 시 선제적 갱신 타이머 시작
          startProactiveRefresh(data.result.expires_in || 7200);
        }
      } catch {
        // ignore
      } finally {
        setSessionLoading(false);
      }
    };
    loadSession();

    // 탭이 다시 활성화되면 토큰 상태 확인 후 필요 시 갱신.
    // 여러 탭이 한꺼번에 활성화될 때 각 탭이 refresh 를 쏘면 회전된 토큰이 재제출되어
    // 서버가 재사용으로 오판하므로 디바운스된 진입점을 쓴다.
    const onVisibilityChange = () => {
      if (document.visibilityState === "visible") {
        refreshOnVisible();
      }
    };
    document.addEventListener("visibilitychange", onVisibilityChange);

    return () => {
      document.removeEventListener("visibilitychange", onVisibilityChange);
      stopProactiveRefresh();
    };
  }, [clientId]);

  const onLoginSuccess = async (credentialResponse) => {
    // credentialResponse 자체는 로그로 남기지 않는다. credential 은 /auth/google 에
    // 그대로 제출하면 세션이 발급되는 Google ID Token 이라, 콘솔에 찍히는 순간
    // 재사용 가능한 자격증명이 브라우저 로그에 남는다.
    console.log("Google Login Success");

    try {
      // 백엔드에서 Google ID Token 검증
      const res = await fetch(getApiUrlPrefix() + "/auth/google", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        credentials: "include",
        body: JSON.stringify({ credential: credentialResponse.credential }),
      });

      if (!res.ok) throw new Error(`API Error: ${res.status}`);

      const data = await res.json();
      console.log("Google auth verified:", data);

      if (data.role) {
        setName(data.name || "Unknown");
        setEmail(data.email || "");
        setPicture(data.picture || "");
        setLogin(true);
        setRole(data.role);
        if (data.expires_in) startProactiveRefresh(data.expires_in);
      }
    } catch (error) {
      console.error("Error verifying Google token:", error);
      alert("Google 로그인 처리 중 오류가 발생했습니다.");
    }
  };

  const logout = async () => {
    console.log("Logging out...");
    stopProactiveRefresh();
    setLogin(false);
    setRole(null);
    setName("");
    setEmail("");
    setPicture("");

    try {
      await fetch(getApiUrlPrefix() + "/auth/logout", {
        method: "POST",
        credentials: "include",
      });
      /* v8 ignore next 3 -- logout network failure is intentionally ignored. */
    } catch {
      // ignore
    }

    googleLogout();
    console.log("Google Logout Completed");
  };

  const renderContent = () => {
    if (sessionLoading) {
      return null;
    }

    if (!login) {
      return (
        <div
          style={{
            display: "flex",
            justifyContent: "center",
            alignItems: "center",
            minHeight: "calc(100vh - 56px)",
          }}
        >
          <GoogleLogin
            onSuccess={onLoginSuccess}
            onError={() => alert("Google 로그인 실패")}
            theme="filled_blue"
            size="large"
            shape="pill"
            text="signin_with"
            locale="ko"
          />
        </div>
      );
    }

    /* v8 ignore next 5 -- authenticated sessions without role are rejected by session loading. */
    if (!role) {
      return (
        <div>{name}님으로 로그인하셨습니다. 서비스 접근 권한이 없습니다.</div>
      );
    }

    if (role === "viewer" && !isViewerAllowedPath(location.pathname)) {
      return <Navigate to="/" replace />;
    }

    return (
      <Outlet
        context={{
          searchResults,
          hasSearched,
          role,
          searchTotal,
          handleLoadMore,
          searchLoading,
        }}
      />
    );
  };

  return (
    <GoogleOAuthProvider clientId={clientId}>
      <div>
        <Navbar bg="light" expand="sm">
          <Navbar.Brand href="/">
            <img src="/book.png" alt="Text" width="32" height="32" />
          </Navbar.Brand>
          <Navbar.Toggle aria-controls="basic-navbar-nav" />
          <Navbar.Collapse id="basic-navbar-nav">
            <Nav className="me-auto my-2 my-lg-0 align-items-sm-center">
              {role && (
                <>
                  <Nav.Link href="/book-view">책</Nav.Link>
                  <span className="text-muted d-none d-sm-inline mx-1">|</span>
                  <Nav.Link href="/book-latest">최신 책</Nav.Link>
                  <span className="text-muted d-none d-sm-inline mx-1">|</span>
                  <Nav.Link href="/comics-view">만화</Nav.Link>
                  <span className="text-muted d-none d-sm-inline mx-1">|</span>
                  <Nav.Link href="/comics-latest">최신 만화</Nav.Link>
                </>
              )}
              {role === "admin" && (
                <>
                  <span className="text-muted d-none d-sm-inline mx-1">|</span>
                  <Nav.Link href="/book-edit">책 편집</Nav.Link>
                  <span className="text-muted d-none d-sm-inline mx-1">|</span>
                  <Nav.Link href="/comics-edit">만화 편집</Nav.Link>
                  <span className="text-muted d-none d-sm-inline mx-1">|</span>
                  <Nav.Link href="/admin">관리</Nav.Link>
                </>
              )}
            </Nav>
            <div className="d-flex align-items-center ms-auto">
              {role && (
                <Form
                  onSubmit={(e) => {
                    e.preventDefault();
                    handleSearch();
                  }}
                  className="me-2"
                >
                  <InputGroup>
                    <FormControl
                      type="text"
                      placeholder="키워드"
                      className="mr-sm-2"
                      style={{ maxWidth: "180px" }}
                      value={searchKeyword}
                      onChange={(e) => setSearchKeyword(e.target.value)}
                    />
                    <Button
                      type="button"
                      variant="outline-success"
                      size="sm"
                      onClick={handleSearch}
                      disabled={searchInProgress}
                    >
                      검색
                      <FontAwesomeIcon
                        icon={searchInProgress ? faSpinner : faSearch}
                        spin={searchInProgress}
                      />
                    </Button>
                  </InputGroup>
                </Form>
              )}
              {login && (
                <Dropdown align="end">
                  <Dropdown.Toggle
                    as="div"
                    style={{ cursor: "pointer", display: "inline-block" }}
                  >
                    {picture ? (
                      <img
                        src={picture}
                        alt={email}
                        title={email}
                        width="38"
                        height="38"
                        className="rounded-circle"
                        style={{ border: "1px solid #cccccc" }}
                      />
                    ) : (
                      <FontAwesomeIcon icon={faUser} size="lg" />
                    )}
                  </Dropdown.Toggle>
                  <Dropdown.Menu>
                    <Dropdown.Item onClick={logout}>로그아웃</Dropdown.Item>
                  </Dropdown.Menu>
                </Dropdown>
              )}
            </div>
          </Navbar.Collapse>
        </Navbar>

        <div className="container ps-0">{renderContent()}</div>
      </div>
    </GoogleOAuthProvider>
  );
}
