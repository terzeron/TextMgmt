import { useEffect, useState } from "react";
import { useOutletContext } from "react-router-dom";
import PropTypes from "prop-types";

import "bootstrap/dist/css/bootstrap.min.css";
import { Alert, Container } from "react-bootstrap";

import { rawJsonGetReq } from "./Common.js";
import SearchResult from "./SearchResult";

const LATEST_ITEM_LIMIT = 100;

const LATEST_CONFIG = {
  book: {
    apiPrefix: "",
    basePath: "/book-view",
    title: "최신 책",
    emptyMessage: "최신 책이 없습니다.",
    errorMessage: "최신 책 목록을 불러오지 못했습니다.",
    containerId: "latest-books",
  },
  comic: {
    apiPrefix: "/comics",
    basePath: "/comics-view",
    title: "최신 만화",
    emptyMessage: "최신 만화가 없습니다.",
    errorMessage: "최신 만화 목록을 불러오지 못했습니다.",
    containerId: "latest-comics",
  },
};

export default function LatestBooks({ contentType = "book" }) {
  const config = LATEST_CONFIG[contentType] || LATEST_CONFIG.book;
  const outletContext = useOutletContext();
  const role = outletContext?.role;
  const [items, setItems] = useState([]);
  const [loading, setLoading] = useState(true);
  const [errorMessage, setErrorMessage] = useState("");

  useEffect(() => {
    setLoading(true);
    setErrorMessage("");
    rawJsonGetReq(
      `${config.apiPrefix}/latest?limit=${LATEST_ITEM_LIMIT}`,
      (data) => {
        if (data.status === "success") {
          setItems(data.result || []);
        } else {
          setItems([]);
          setErrorMessage(config.errorMessage);
        }
      },
      () => {
        setItems([]);
        setErrorMessage(config.errorMessage);
      },
      () => setLoading(false),
    );
  }, [config]);

  return (
    <Container id={config.containerId} className="ps-0 pe-0">
      {errorMessage && (
        <Alert variant="danger" className="mb-2">
          {errorMessage}
        </Alert>
      )}
      <SearchResult
        results={items}
        role={role}
        showEditButton={role === "admin"}
        basePath={config.basePath}
        title={config.title}
        emptyMessage={loading ? "로딩 중..." : config.emptyMessage}
      />
    </Container>
  );
}

LatestBooks.propTypes = {
  contentType: PropTypes.oneOf(["book", "comic"]),
};
