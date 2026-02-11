import { useEffect, useState, useCallback } from "react";
import { Outlet, useLocation, Navigate } from "react-router-dom";
import { GoogleOAuthProvider, GoogleLogin, googleLogout } from "@react-oauth/google";
import { Button, Form, FormControl, InputGroup, Nav, Navbar, Dropdown } from "react-bootstrap";
import { FontAwesomeIcon } from "@fortawesome/react-fontawesome";
import { faSearch, faUser } from "@fortawesome/free-solid-svg-icons";
import { rawJsonGetReq, getApiUrlPrefix } from "./Common.js";
import { determineRole, isViewerAllowedPath } from "./auth.js";

export default function Navigation() {
    const clientId = window.__ENV__?.['VITE_GOOGLE_CLIENT_ID'] || import.meta.env.VITE_GOOGLE_CLIENT_ID;
    const adminEmail = window.__ENV__?.['VITE_ADMIN_EMAIL'] || import.meta.env.VITE_ADMIN_EMAIL;
    const allowedEmailsRaw = window.__ENV__?.['VITE_ALLOWED_EMAILS'] || import.meta.env.VITE_ALLOWED_EMAILS || '';
    const allowedEmails = allowedEmailsRaw ? allowedEmailsRaw.split(',').map(e => e.trim()).filter(Boolean) : [];

    const [login, setLogin] = useState(false);
    const [role, setRole] = useState(null); // 'admin' | 'viewer' | null
    const [name, setName] = useState('');
    const [email, setEmail] = useState('');
    const [picture, setPicture] = useState('');
    const [searchKeyword, setSearchKeyword] = useState('');
    const [searchResults, setSearchResults] = useState([]);
    const [hasSearched, setHasSearched] = useState(false);
    const [searchTotal, setSearchTotal] = useState(0);
    const [searchLoading, setSearchLoading] = useState(false);
    const [hiddenCategories, setHiddenCategories] = useState([]);

    const location = useLocation();

    // viewer일 때 비노출 카테고리 목록 로드
    useEffect(() => {
        if (role === 'viewer') {
            rawJsonGetReq('/hidden-categories',
                (data) => {
                    if (data.status === 'success') {
                        setHiddenCategories(data.result || []);
                    }
                },
                () => { setHiddenCategories([]); }
            );
        } else {
            setHiddenCategories([]);
        }
    }, [role]);

    const buildSearchUrl = (keyword, offset, limit) => {
        let url = `/search/${encodeURIComponent(keyword)}?offset=${offset}&limit=${limit}`;
        if (hiddenCategories.length > 0) {
            url += `&exclude_categories=${encodeURIComponent(hiddenCategories.join(','))}`;
        }
        return url;
    };

    // 검색 실행 로직을 함수로 추출
    const handleSearch = () => {
        if (searchKeyword) {
            setHasSearched(true);
            rawJsonGetReq(
                buildSearchUrl(searchKeyword, 0, 10),
                (data) => {
                    if (data.status === 'success') {
                        setSearchResults(data.result || []);
                        setSearchTotal(data.total || 0);
                    }
                },
                (error) => { console.error(error); }
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
                if (data.status === 'success' && data.result) {
                    setSearchResults(prev => [...prev, ...data.result]);
                    setSearchTotal(data.total || 0);
                }
                setSearchLoading(false);
            },
            (error) => {
                console.error(error);
                setSearchLoading(false);
            }
        );
    }, [searchKeyword, searchResults.length, searchLoading, hiddenCategories]);

    useEffect(() => {
        if (!clientId) {
            console.error("The environment variable VITE_GOOGLE_CLIENT_ID is not set.");
            return;
        }
        if (!adminEmail) {
            console.error("The environment variable VITE_ADMIN_EMAIL is not set.");
            return;
        }

        // localStorage에서 저장된 정보로 로그인 상태 복원
        const storedEmail = localStorage.getItem('email');
        const storedName = localStorage.getItem('name');
        const storedPicture = localStorage.getItem('picture');

        if (storedEmail) {
            const storedRole = determineRole(storedEmail, adminEmail, allowedEmails);
            setLogin(true);
            setRole(storedRole);
            setName(storedName || '');
            setEmail(storedEmail || '');
            setPicture(storedPicture || '');
        }
    }, [adminEmail, clientId]);

    const onLoginSuccess = async (credentialResponse) => {
        console.log('Google Login Success:', credentialResponse);

        try {
            // 백엔드에서 Google ID Token 검증
            const res = await fetch(getApiUrlPrefix() + '/auth/google', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ credential: credentialResponse.credential }),
            });

            if (!res.ok) throw new Error(`API Error: ${res.status}`);

            const data = await res.json();
            console.log('Google auth verified:', data);

            if (data.email) {
                const profileName = data.name || 'Unknown';
                const profileEmail = data.email || '';
                const profilePicture = data.picture || '';

                setName(profileName);
                setEmail(profileEmail);
                setPicture(profilePicture);
                const userRole = determineRole(profileEmail, adminEmail, allowedEmails);
                setLogin(true);
                setRole(userRole);

                localStorage.setItem('name', profileName);
                localStorage.setItem('email', profileEmail);
                localStorage.setItem('picture', profilePicture);
            }
        } catch (error) {
            console.error('Error verifying Google token:', error);
            alert('Google 로그인 처리 중 오류가 발생했습니다.');
        }
    };

    const logout = () => {
        console.log('Logging out...');
        setLogin(false);
        setRole(null);
        setName('');
        setEmail('');
        setPicture('');

        localStorage.removeItem('name');
        localStorage.removeItem('email');
        localStorage.removeItem('picture');

        googleLogout();
        console.log('Google Logout Completed');
    };

    const renderContent = () => {
        if (!login) {
            return (
                <GoogleLogin
                    onSuccess={onLoginSuccess}
                    onError={() => alert("Google 로그인 실패")}
                />
            );
        }

        if (!role) {
            return <div>{name}님으로 로그인하셨습니다. 서비스 접근 권한이 없습니다.</div>;
        }

        if (role === 'viewer' && (location.pathname === '/' || !isViewerAllowedPath(location.pathname))) {
            return <Navigate to="/view" replace />;
        }

        return <Outlet context={{ searchResults, hasSearched, role, searchTotal, handleLoadMore, searchLoading }} />;
    };

    return (
        <GoogleOAuthProvider clientId={clientId}>
            <div>
                <Navbar bg="light" expand="sm">
                    <Navbar.Brand href="/">Text</Navbar.Brand>
                    <Navbar.Toggle aria-controls="basic-navbar-nav" />
                    <Navbar.Collapse id="basic-navbar-nav">
                        <Nav className="me-auto my-2 my-lg-0" style={{ maxHeight: '100px' }} navbarScroll>
                            {role === 'admin' && <Nav.Link href="/edit">편집</Nav.Link>}
                            {role && <Nav.Link href="/view">조회</Nav.Link>}
                            {role === 'admin' && <Nav.Link href="/admin">관리</Nav.Link>}
                        </Nav>
                        <div className="d-flex align-items-center ms-auto">
                            {role && (
                                <Form onSubmit={e => { e.preventDefault(); handleSearch(); }} className="me-2">
                                    <InputGroup>
                                        <FormControl type="text" placeholder="키워드" className="mr-sm-2" value={searchKeyword} onChange={e => setSearchKeyword(e.target.value)} />
                                        <Button type="button" variant="outline-success" size="sm" onClick={handleSearch}>
                                            검색<FontAwesomeIcon icon={faSearch} />
                                        </Button>
                                    </InputGroup>
                                </Form>
                            )}
                            {login && (
                                <Dropdown align="end">
                                    <Dropdown.Toggle as="div" style={{ cursor: 'pointer', display: 'inline-block' }}>
                                        {picture ? (
                                            <img src={picture} alt={email} title={email} width="38" height="38" className="rounded-circle" style={{ border: '1px solid #cccccc' }} />
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

                <div className="container ps-0">
                    {renderContent()}
                </div>
            </div>
        </GoogleOAuthProvider>
    );
}
