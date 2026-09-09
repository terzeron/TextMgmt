import { useState } from "react";
import { useOutletContext } from "react-router-dom";

import "./Edit.css";
import "bootstrap/dist/css/bootstrap.min.css";
import "./Admin.css";
import { Nav, Tab } from "react-bootstrap";

import CategoryAdmin from "./CategoryAdmin";
import LoginSessionAdmin from "./LoginSessionAdmin";
import ViewHistoryAdmin from "./ViewHistoryAdmin";

const SESSION_TAB = "login-session";
const VIEW_HISTORY_TAB = "view-history";
const BOOK_TAB = "book-category";
const COMIC_TAB = "comic-category";

// 탭 띠와 패널이 같은 배열을 읽어 순서가 어긋나지 않게 한다.
const SUBTABS = [
  {
    key: SESSION_TAB,
    title: "로그인 세션 관리",
    render: () => <LoginSessionAdmin />,
  },
  {
    key: VIEW_HISTORY_TAB,
    title: "사용자별 조회 목록",
    render: () => <ViewHistoryAdmin />,
  },
  {
    key: BOOK_TAB,
    title: "책 카테고리 관리",
    render: () => <CategoryAdmin contentType="book" />,
  },
  {
    key: COMIC_TAB,
    title: "만화 카테고리 관리",
    render: () => <CategoryAdmin contentType="comic" />,
  },
];

export default function Admin() {
  useOutletContext();
  const [activeTab, setActiveTab] = useState(SESSION_TAB);

  return (
    <Tab.Container
      id="admin-subtabs"
      activeKey={activeTab}
      onSelect={(key) => setActiveTab(key || SESSION_TAB)}
    >
      {/* 최상단 탭(Navigation.jsx)과 같이 nav 요소로 띠를 만든다. Tabs는 ul을
          강제하므로 Tab.Container + Nav 로 낮춰 조립한다. TabContext 안의 Nav 는
          role="tablist" 와 방향키 이동을 그대로 유지한다. */}
      <Nav
        as="nav"
        variant="tabs"
        id="admin-subtabs"
        className="admin-modern-tabs mb-3"
      >
        {SUBTABS.map(({ key, title }) => (
          <Nav.Link as="button" type="button" key={key} eventKey={key}>
            {title}
          </Nav.Link>
        ))}
      </Nav>
      <Tab.Content>
        {SUBTABS.map(({ key, render }) => (
          <Tab.Pane key={key} eventKey={key}>
            {activeTab === key && render()}
          </Tab.Pane>
        ))}
      </Tab.Content>
    </Tab.Container>
  );
}
