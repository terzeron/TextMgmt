import logo from './logo.svg';
import './App.css';
import 'bootstrap/dist/css/bootstrap.min.css';
import {Button, Navbar, Nav, Form, FormControl, InputGroup} from 'react-bootstrap';
import {BrowserRouter as Router, Switch, Route} from "react-router-dom";
import Home from './Home';
import Edit from './Edit';
import View from './View';
import {FontAwesomeIcon} from "@fortawesome/react-fontawesome";
import {faSearch, faTrash} from "@fortawesome/free-solid-svg-icons";


function App() {
    return (
        <div className="App">
            <Router>
                <Navbar bg="light" expand="sm">
                    <Navbar.Brand href="/">Text</Navbar.Brand>
                    <Navbar.Toggle aria-controls="basic-navbar-nav"/>
                    <Navbar.Collapse id="basic-navbar-nav">
                        <Nav className="me-auto my-2 my-lg-0" style={{maxHeight: '100px'}} navbarScroll>
                            <Nav.Link href="/edit">편집</Nav.Link>
                            <Nav.Link href="/view">조회</Nav.Link>
                            <Nav.Link onClick={login}>로그인</Nav.Link>
                            <Nav.Link onClick={logout}>로그아웃</Nav.Link>
                        </Nav>
                        <Form>
                            <InputGroup>
                                <FormControl type="text" placeholder="Search" className="mr-sm-2"/>
                                <Button variant="outline-success" size="sm">검색 <FontAwesomeIcon icon={faSearch} /></Button>
                            </InputGroup>
                        </Form>
                    </Navbar.Collapse>
                </Navbar>

                <Switch>
                    <Route exact path="/">
                        <Home/>
                    </Route>
                    <Route path="/edit">
                        <Edit/>
                    </Route>
                    <Route path="/view">
                        <View/>
                    </Route>
                </Switch>

            </Router>
        </div>
    );
}

function login() {
    console.log("logged in");
}

function logout() {
    console.log("logged out");
}

export default App;


