import { useState } from "react";
import { useOutletContext } from "react-router-dom";

import "./Edit.css";
import "bootstrap/dist/css/bootstrap.min.css";
import "./Admin.css";
import { Tab, Tabs } from "react-bootstrap";

import CategoryAdmin from "./CategoryAdmin";
import LoginSessionAdmin from "./LoginSessionAdmin";
import ViewHistoryAdmin from "./ViewHistoryAdmin";

const SESSION_TAB = "login-session";
const VIEW_HISTORY_TAB = "view-history";
const BOOK_TAB = "book-category";
const COMIC_TAB = "comic-category";

export default function Admin() {
  useOutletContext();
  const [activeTab, setActiveTab] = useState(SESSION_TAB);

  return (
    <Tabs
      id="admin-subtabs"
      activeKey={activeTab}
      onSelect={(key) => setActiveTab(key || SESSION_TAB)}
      className="admin-modern-tabs mb-3"
    >
      <Tab eventKey={SESSION_TAB} title="로그인 세션 관리">
        {activeTab === SESSION_TAB && <LoginSessionAdmin />}
      </Tab>
      <Tab eventKey={VIEW_HISTORY_TAB} title="사용자별 조회 목록">
        {activeTab === VIEW_HISTORY_TAB && <ViewHistoryAdmin />}
      </Tab>
      <Tab eventKey={BOOK_TAB} title="책 카테고리 관리">
        {activeTab === BOOK_TAB && <CategoryAdmin contentType="book" />}
      </Tab>
      <Tab eventKey={COMIC_TAB} title="만화 카테고리 관리">
        {activeTab === COMIC_TAB && <CategoryAdmin contentType="comic" />}
      </Tab>
    </Tabs>
  );
}
