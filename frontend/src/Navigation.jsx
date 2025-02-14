import { useEffect, useState } from "react";
import { Outlet } from "react-router-dom";
import FacebookLogin, { FacebookLoginClient } from "@greatsumini/react-facebook-login";
import { Button, Form, FormControl, InputGroup, Nav, Navbar } from "react-bootstrap";
import { FontAwesomeIcon } from "@fortawesome/react-fontawesome";
import { faSearch, faUser } from "@fortawesome/free-solid-svg-icons";
import { faFacebook } from "@fortawesome/free-brands-svg-icons";
import { getApiUrlPrefix } from "./Common.js";

export default function Navigation() {
    const appId = import.meta.env.VITE_FACEBOOK_APP_ID;
    const adminEmail = import.meta.env.VITE_ADMIN_EMAIL;
    const [login, setLogin] = useState(false);
    const [authorized, setAuthorized] = useState(false);
    const [name, setName] = useState('');
    const [email, setEmail] = useState('');
    const [picture, setPicture] = useState('');

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
        const profilePicture = response.picture?.data?.url || '/default.jpg';

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
                <Navbar.Toggle aria-controls="basic-navbar-nav"/>
                <Navbar.Collapse id="basic-navbar-nav">
                    <Nav className="me-auto my-2 my-lg-0" style={{maxHeight: '100px'}} navbarScroll>
                        <Nav.Link href="/edit">편집</Nav.Link>
                        <Nav.Link href="/view">조회</Nav.Link>
                        {login && <Nav.Link onClick={logout}>로그아웃</Nav.Link>}
                    </Nav>
                    <Form>
                        <InputGroup>
                            <FormControl type="text" placeholder="키워드" className="mr-sm-2"/>
                            <Button variant="outline-success" size="sm" onClick={() => alert('검색 버튼 클릭됨')}>
                                검색
                                <FontAwesomeIcon icon={faSearch}/>
                            </Button>
                        </InputGroup>
                    </Form>
                    <div className="ms-2">
                        {login && picture ? (
                            <img src={picture} alt={"profile image of " + email} title={email} width="38" height="38" className="rounded-circle" style={{ 'border': '1px solid #cccccc' }} />
                        ) : (
                            <FontAwesomeIcon icon={faUser} />
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
                        Login with Facebook <FontAwesomeIcon icon={faFacebook}/>
                    </FacebookLogin>
                )}
                {authorized && <Outlet />}
                {login && !authorized && <div>{name}님으로 로그인하셨습니다. 권한 부족</div>}
            </div>
        </div>
    );
}
