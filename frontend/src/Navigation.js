import {Button, Form, FormControl, InputGroup, Nav, Navbar} from "react-bootstrap";
import {FontAwesomeIcon} from "@fortawesome/react-fontawesome";
import {faSearch} from "@fortawesome/free-solid-svg-icons";
import React from "react";
import {Outlet} from "react-router-dom";

export default function Navigation() {
  const searchClicked = () => {
    alert('검색 버튼 클릭됨');
  };

  const login = () => {
    console.log('logged in');
  };

  const logout = () => {
    console.log('logged out');
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
            <Nav.Link onClick={login}>로그인</Nav.Link>
            <Nav.Link onClick={logout}>로그아웃</Nav.Link>
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
        </Navbar.Collapse>
      </Navbar>

      <Outlet/>
    </div>
  );
};