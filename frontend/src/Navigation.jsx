import {useEffect, useState} from "react";
import {Outlet} from "react-router-dom";

import FacebookLogin, {FacebookLoginClient} from "@greatsumini/react-facebook-login";
import {Button, Form, FormControl, InputGroup, Nav, Navbar} from "react-bootstrap";
import {FontAwesomeIcon} from "@fortawesome/react-fontawesome";
import {faSearch, faUser} from "@fortawesome/free-solid-svg-icons";
import {faFacebook} from "@fortawesome/free-brands-svg-icons";

export default function Navigation() {
    const appId = import.meta.env.VITE_FACEBOOK_APP_ID;
    const adminEmail = import.meta.env.VITE_ADMIN_EMAIL;
    const [login, setLogin] = useState(false);
    const [authorized, setAuthorized] = useState(false);
    const [name, setName] = useState('');
    const [, setEmail] = useState('');
    const [picture, setPicture] = useState('');

    const searchClicked = () => {
        alert('검색 버튼 클릭됨');
    };

    useEffect(() => {
        if (!appId) {
            console.error("The environment variable VITE_FACEBOOK_APP_ID is not set.");
            return;
        }
        if (!adminEmail) {
            console.log("The environment variable VITE_ADMIN_EMAIL is not set.");
            return;
        }

        if (sessionStorage.getItem('accessToken') && sessionStorage.getItem('email') === adminEmail) {
            setLogin(true);
            setAuthorized(true);
            setName(sessionStorage.getItem('name'));
            setEmail(sessionStorage.getItem('email'));
            setPicture(sessionStorage.getItem('picture'));
        }
    }, [adminEmail, appId]);

    const onLoginSuccess = (response) => {
        setLogin(true);
        sessionStorage.setItem('accessToken', response.accessToken);
        console.log('Login Success!', response);
    };

    const onLoginFailure = (error) => {
        setLogin(false);
        setAuthorized(false);
        console.log('Login Failed!', error);
    };

    const onProfileSuccess = (response) => {
        console.log('Get Profile Success!', response);
        setName(response.name);
        setEmail(response.email);
        setPicture(response.picture.data.url);
        if (response.email === adminEmail) {
            setAuthorized(true);
            sessionStorage.setItem('name', response.name);
            sessionStorage.setItem('email', response.email);
            sessionStorage.setItem('picture', response.picture.data.url);
        }
    };

    const logout = () => {
        console.log('logout():');
        setLogin(false);
        setAuthorized(false);
        setName('');
        setEmail('');
        setPicture('');
        sessionStorage.removeItem('accessToken');
        sessionStorage.removeItem('name');
        sessionStorage.removeItem('email');
        sessionStorage.removeItem('picture');
        FacebookLoginClient.logout(() => {
            console.log('FacebookLoginClient.logout() completed');
            console.log(sessionStorage);
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
                        {
                            login && <Nav.Link onClick={logout}>로그아웃</Nav.Link>
                        }
                        {
                            !login && <Nav.Link href="/">로그인</Nav.Link>
                        }
                    </Nav>
                    <Form>
                        <InputGroup>
                            <FormControl type="text" placeholder="키워드" className="mr-sm-2"/>
                            <Button variant="outline-success" size="sm" onClick={searchClicked}>
                                검색
                                <FontAwesomeIcon icon={faSearch}/>
                            </Button>
                        </InputGroup>
                    </Form>
                    <div className="ms-2">
                        {
                            picture && <img src={picture} alt="profile" width="38" height="38" className="rounded-circle" style={{'border': '1px solid #cccccc'}}/>
                        }
                        {
                            !picture && <FontAwesomeIcon icon={faUser}/>
                        }
                    </div>
                </Navbar.Collapse>
            </Navbar>

            <div className="container ps-0">
                {
                    !login &&
                    <FacebookLogin
                        appId={appId}
                        onSuccess={onLoginSuccess}
                        onFail={onLoginFailure}
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
                }
                {
                    authorized &&
                    <div>
                        <Outlet/>
                    </div>
                }
                {
                    login && !authorized &&
                    <div>{name}님으로 로그인하셨습니다. 권한 부족</div>
                }
            </div>
        </div>
    );
}