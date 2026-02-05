import { useEffect, useState } from "react";
import { Outlet } from "react-router-dom";
import FacebookLogin, { FacebookLoginClient } from "@greatsumini/react-facebook-login";
import { Button, Form, FormControl, InputGroup, Nav, Navbar, Dropdown } from "react-bootstrap";
import { FontAwesomeIcon } from "@fortawesome/react-fontawesome";
import { faSearch, faUser } from "@fortawesome/free-solid-svg-icons";
import { faFacebook } from "@fortawesome/free-brands-svg-icons";
import { getApiUrlPrefix, jsonGetReq } from "./Common.js";

export default function Navigation() {
    const appId = window.__ENV__?.['VITE_FACEBOOK_APP_ID'] || import.meta.env.VITE_FACEBOOK_APP_ID;
    const adminEmail = window.__ENV__?.['VITE_ADMIN_EMAIL'] || import.meta.env.VITE_ADMIN_EMAIL;
    const [login, setLogin] = useState(false);
    const [authorized, setAuthorized] = useState(false);
    const [name, setName] = useState('');
    const [email, setEmail] = useState('');
    const [picture, setPicture] = useState('');
    const [searchKeyword, setSearchKeyword] = useState('');
    const [searchResults, setSearchResults] = useState([]);
    const [hasSearched, setHasSearched] = useState(false);
    // 검색 실행 로직을 함수로 추출
    const handleSearch = () => {
        if (searchKeyword) {
            setHasSearched(true);
            jsonGetReq(
                `/search/${encodeURIComponent(searchKeyword)}`,
                null,
                (data) => { setSearchResults(data); },
                (error) => { console.error(error); }
            );
        }
    };

    useEffect(() => {
        if (!appId) {
            console.error("The environment variable VITE_FACEBOOK_APP_ID is not set.");
            return;
        }
        if (!adminEmail) {
            console.error("The environment variable VITE_ADMIN_EMAIL is not set.");
            return;
        }

        // 🔹 localStorage에서 저장된 정보 가져오기
        const storedToken = localStorage.getItem('longLivedToken');
        const storedEmail = localStorage.getItem('email');
        const storedName = localStorage.getItem('name');
        const storedPicture = localStorage.getItem('picture');

        if (storedToken) {
            setLogin(true);
            if (storedEmail === adminEmail) {
                setAuthorized(true);
                setName(storedName || '');
                setEmail(storedEmail || '');
                setPicture(storedPicture || '');
            }
        }
    }, [adminEmail, appId]);

    const onLoginSuccess = async (response) => {
        console.log('Facebook Login Success:', response);

        try {
            // changed short-lived token(access token) to long-lived token using backend
            const res = await fetch(getApiUrlPrefix() + '/auth/facebook', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ accessToken: response.accessToken }),
            });

            if (!res.ok) throw new Error(`API Error: ${res.status}`);

            const data = await res.json();
            console.log('Long-lived token received:', data.longLivedToken);

            if (data.longLivedToken) {
                localStorage.setItem('longLivedToken', data.longLivedToken);
                localStorage.setItem('accessToken', response.accessToken);
                setLogin(true);
            }
        } catch (error) {
            console.error('Error exchanging token:', error);
            alert('Facebook 로그인 처리 중 오류가 발생했습니다.');
        }
    };

    const onProfileSuccess = (response) => {
        console.log('Facebook Profile Success:', response);

        const profileName = response.name || 'Unknown';
        const profileEmail = response.email || '';
        // Facebook Graph API의 영구적인 프로필 이미지 URL 사용 (임시 URL은 만료됨)
        const profilePicture = response.id
            ? `https://graph.facebook.com/${response.id}/picture?width=50&height=50`
            : '/default.jpg';

        setName(profileName);
        setEmail(profileEmail);
        setPicture(profilePicture);

        if (profileEmail === adminEmail) {
            setAuthorized(true);
            localStorage.setItem('name', profileName);
            localStorage.setItem('email', profileEmail);
            localStorage.setItem('picture', profilePicture);
        }
    };

    const logout = () => {
        console.log('Logging out...');
        setLogin(false);
        setAuthorized(false);
        setName('');
        setEmail('');
        setPicture('');

        localStorage.removeItem('longLivedToken');
        localStorage.removeItem('accessToken');
        localStorage.removeItem('name');
        localStorage.removeItem('email');
        localStorage.removeItem('picture');

        FacebookLoginClient.logout(() => {
            console.log('Facebook Logout Completed');
        });
    };

    return (
        <div>
            <Navbar bg="light" expand="sm">
                <Navbar.Brand href="/">Text</Navbar.Brand>
                <Navbar.Toggle aria-controls="basic-navbar-nav" />
                <Navbar.Collapse id="basic-navbar-nav">
                    <Nav className="me-auto my-2 my-lg-0" style={{ maxHeight: '100px' }} navbarScroll>
                        <Nav.Link href="/edit">편집</Nav.Link>
                        <Nav.Link href="/view">조회</Nav.Link>
                    </Nav>
                    <div className="d-flex align-items-center ms-auto">
                        <Form onSubmit={e => { e.preventDefault(); handleSearch(); }} className="me-2">
                            <InputGroup>
                                <FormControl type="text" placeholder="키워드" className="mr-sm-2" value={searchKeyword} onChange={e => setSearchKeyword(e.target.value)} />
                                <Button type="button" variant="outline-success" size="sm" onClick={handleSearch}>
                                    검색<FontAwesomeIcon icon={faSearch} />
                                </Button>
                            </InputGroup>
                        </Form>
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
                {!login && (
                    <FacebookLogin
                        appId={appId}
                        onSuccess={onLoginSuccess}
                        onFail={() => alert("Facebook 로그인 실패")}
                        onProfileSuccess={onProfileSuccess}
                        style={{
                            backgroundColor: '#4267b2',
                            color: '#fff',
                            fontSize: '16px',
                            padding: '10px',
                            border: 'none',
                            borderRadius: '4px',
                        }}
                    >
                        Login with Facebook <FontAwesomeIcon icon={faFacebook} />
                    </FacebookLogin>
                )}
                {authorized && <Outlet context={{ searchResults, hasSearched }} />}
                {login && !authorized && <div>{name}님으로 로그인하셨습니다. 권한 부족</div>}
            </div>
        </div>
    );
}
