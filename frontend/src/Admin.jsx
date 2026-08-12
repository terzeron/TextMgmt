import { useState } from "react";
import { useOutletContext } from "react-router-dom";

import "./Edit.css";
import "bootstrap/dist/css/bootstrap.min.css";
import { Tab, Tabs } from "react-bootstrap";

import CategoryAdmin from "./CategoryAdmin";
import LoginSessionAdmin from "./LoginSessionAdmin";

const BOOK_TAB = "book-category";
const COMIC_TAB = "comic-category";
const SESSION_TAB = "login-session";

export default function Admin() {
  useOutletContext();
  const [activeTab, setActiveTab] = useState(BOOK_TAB);

  return (
    <Tabs
      id="admin-subtabs"
      activeKey={activeTab}
      onSelect={(key) => setActiveTab(key || BOOK_TAB)}
      className="mb-3"
    >
      <Tab eventKey={BOOK_TAB} title="책 카테고리 관리">
        {activeTab === BOOK_TAB && (
          <CategoryAdmin contentType="book" title="책 카테고리 관리" />
        )}
      </Tab>
      <Tab eventKey={COMIC_TAB} title="만화 카테고리 관리">
        {activeTab === COMIC_TAB && (
          <CategoryAdmin contentType="comic" title="만화 카테고리 관리" />
        )}
      </Tab>
      <Tab eventKey={SESSION_TAB} title="로그인 세션 관리">
        {activeTab === SESSION_TAB && <LoginSessionAdmin />}
      </Tab>
    </Tabs>
  );
}
