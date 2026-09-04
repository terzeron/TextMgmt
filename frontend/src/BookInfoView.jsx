import { useEffect, useState } from "react";
import PropTypes from "prop-types";

import "./Edit.css";
import "bootstrap/dist/css/bootstrap.min.css";

import { Button, Col, Form, InputGroup, Row } from "react-bootstrap";
import { FontAwesomeIcon } from "@fortawesome/react-fontawesome";
import {
  faClockRotateLeft,
  faCut,
  faRotate,
} from "@fortawesome/free-solid-svg-icons";

export default function BookInfoView(props) {
  // bookId, category state는 현재 UI에 사용되지 않으므로 제거
  const [title, setTitle] = useState("");
  const [author, setAuthor] = useState("");
  const [fileType, setFileType] = useState("");
  const [filePath, setFilePath] = useState("");
  const [fileSize, setFileSize] = useState(0);
  const [lineCount, setLineCount] = useState(0);
  const [pageCount, setPageCount] = useState(0);
  const [isbn, setIsbn] = useState("");
  const [isEditEnabled, setIsEditEnabled] = useState(false);
  const [isComposingAuthor, setIsComposingAuthor] = useState(false);
  const [isComposingTitle, setIsComposingTitle] = useState(false);

  useEffect(() => {
    const bookInfo = props.bookInfo || {};
    // book_id와 category는 UI에 표시하지 않아 상태 업데이트 생략
    setTitle(bookInfo["title"] || "");
    setAuthor(bookInfo["author"] || "");
    setFileType(bookInfo["file_type"] || "");
    setFilePath(bookInfo["file_path"] || "");
    setFileSize(bookInfo["file_size"] || 0);
    setLineCount(bookInfo["line_count"] || 0);
    setPageCount(bookInfo["page_count"] || 0);
    setIsbn(bookInfo["isbn"] || "");
    setIsEditEnabled(Boolean(props.isEditEnabled));
  }, [props]);

  return (
    <>
      <Row>
        <Col>
          <Form.Control value={filePath} readOnly disabled />
        </Col>
      </Row>

      <Row>
        <Col xs="2">
          <InputGroup size="sm">
            <InputGroup.Text>종류</InputGroup.Text>
            <Form.Control value={fileType} readOnly disabled />
          </InputGroup>
        </Col>
        <Col xs="4">
          <InputGroup size="sm">
            <InputGroup.Text>ISBN</InputGroup.Text>
            <Form.Control value={isbn || "-"} readOnly disabled />
          </InputGroup>
        </Col>
        <Col xs="3">
          <InputGroup size="sm">
            <InputGroup.Text>크기</InputGroup.Text>
            <Form.Control value={fileSize.toLocaleString()} readOnly disabled />
          </InputGroup>
        </Col>
        <Col xs="3">
          <InputGroup size="sm">
            <InputGroup.Text>분량</InputGroup.Text>
            <Form.Control
              value={
                pageCount > 0
                  ? `${pageCount.toLocaleString()}쪽`
                  : lineCount > 0
                    ? `${lineCount.toLocaleString()}행`
                    : "-"
              }
              readOnly
              disabled
            />
          </InputGroup>
        </Col>
      </Row>

      {isEditEnabled && (
        <Row>
          <InputGroup>
            <InputGroup.Text>저자</InputGroup.Text>
            <Form.Control
              value={author}
              onCompositionStart={() => setIsComposingAuthor(true)}
              onCompositionEnd={(e) => {
                setIsComposingAuthor(false);
                const val = e.target.value;
                setAuthor(val);
                props.onAuthorChange(e);
              }}
              onChange={(e) => {
                const val = e.target.value;
                setAuthor(val);
                if (!isComposingAuthor) {
                  props.onAuthorChange(e);
                }
              }}
            />
            <Button
              variant="outline-secondary"
              className="btn-xs"
              onClick={(e) => {
                props.onCutAuthorButtonClick(e);
              }}
            >
              분할
              <FontAwesomeIcon icon={faCut} />
            </Button>
            <Button
              variant="outline-secondary"
              className="btn-xs"
              onClick={props.onExchangeButtonClick}
            >
              교환
              <FontAwesomeIcon icon={faRotate} />
            </Button>
          </InputGroup>
        </Row>
      )}

      {isEditEnabled && (
        <Row>
          <InputGroup>
            <InputGroup.Text>제목</InputGroup.Text>
            <Form.Control
              value={title}
              onCompositionStart={() => setIsComposingTitle(true)}
              onCompositionEnd={(e) => {
                setIsComposingTitle(false);
                const val = e.target.value;
                setTitle(val);
                props.onTitleChange(e);
              }}
              onChange={(e) => {
                const val = e.target.value;
                setTitle(val);
                if (!isComposingTitle) {
                  props.onTitleChange(e);
                }
              }}
            />
            <Button
              variant="outline-secondary"
              className="btn-xs"
              onClick={(e) => {
                props.onCutTitleButtonClick(e);
              }}
            >
              분할
              <FontAwesomeIcon icon={faCut} />
            </Button>
            <Button
              variant="outline-secondary"
              className="btn-xs"
              onClick={(e) => {
                props.onResetButtonClick(e);
              }}
            >
              복원
              <FontAwesomeIcon icon={faClockRotateLeft} />
            </Button>
          </InputGroup>
        </Row>
      )}
    </>
  );
}

BookInfoView.propTypes = {
  bookInfo: PropTypes.object.isRequired,
  isEditEnabled: PropTypes.bool,
  onTitleChange: PropTypes.func,
  onAuthorChange: PropTypes.func,
  onCutTitleButtonClick: PropTypes.func,
  onCutAuthorButtonClick: PropTypes.func,
  onExchangeButtonClick: PropTypes.func,
  onResetButtonClick: PropTypes.func,
};
