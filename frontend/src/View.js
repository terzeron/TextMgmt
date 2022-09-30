import './View.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import { useState } from 'react';
import {
  Button, Card, Col, Container, Form, InputGroup, Row,
} from 'react-bootstrap';
import { getUrlPrefix, handleFetchErrors } from './Common';
import DirList from './DirList';

export default function View() {
  const [loading, setLoading] = useState(false);
  const [errorMessage, setErrorMessage] = useState(null);

  const [fileMetadata, setFileMetadata] = useState({
    id: '',
    name: '',
  });
  const [fileContent, setFileContent] = useState({
    size: '',
    encoding: '',
    content: '',
  });

  function fileClicked(key, label, props) {
    const fileId = key;
    const dirName = fileId.split('/')[0];
    const fileName = fileId.split('/')[1];
    setFileMetadata({ id: fileId, name: fileName });

    fetch(`${getUrlPrefix()}/dirs/${dirName}/files/${fileName}`)
      .then(handleFetchErrors)
      .then((response) => {
        response.json()
          .then((result) => {
            if (result.status === 'success') {
              setFileContent(result.result);
              setErrorMessage(null);
            } else {
              setErrorMessage(result.error);
              setFileContent({
                size: '',
                encoding: '',
                content: '',
              });
            }
          })
          .catch((err) => {
            setErrorMessage(err.message);
            setFileContent({
              size: '',
              encoding: '',
              content: '',
            });
          });
      })
      .catch((error) => {
        setErrorMessage(error.message);
        setFileContent({
          size: '',
          encoding: '',
          content: '',
        });
      });
  }

  if (loading) return <div>로딩 중..</div>;
  return (
    <Container id="view">
      <Row fluid="true">
        <Col md="3" lg="2" className="ps-0 pe-0">
          <DirList onClickHandler={fileClicked} />
        </Col>

        <Col md="9" lg="10">
          <Row id="top_panel">
            <Col lg="12" className="ps-0 pe-0 me-0">
              <Card>
                <Card.Header>
                  파일 정보
                </Card.Header>
                <Card.Body>
                  <Row>
                    <Col xs="6">
                      <InputGroup>
                        <InputGroup.Text>파일명</InputGroup.Text>
                        <Form.Control defaultValue={fileMetadata.name} readOnly />
                      </InputGroup>
                    </Col>
                    <Col xs="3">
                      <InputGroup>
                        <InputGroup.Text>인코딩</InputGroup.Text>
                        <Form.Control defaultValue={fileContent.encoding} readOnly />
                      </InputGroup>
                    </Col>
                    <Col xs="3">
                      <InputGroup>
                        <InputGroup.Text>크기</InputGroup.Text>
                        <Form.Control defaultValue={fileContent.size} readOnly />
                      </InputGroup>
                    </Col>
                  </Row>
                  <Row className="mt-1">
                    <span>
                      <a href={`/view_single/${fileMetadata.id}`} target="_blank" rel="noreferrer">
                        <Button variant="outline-secondary" size="sm" disabled={!fileMetadata.id}>보기</Button>
                      </a>
                      <a href={`/view/download/${fileMetadata.id}`} target="_blank" rel="noreferrer">
                        <Button variant="outline-secondary" size="sm" disabled={!fileMetadata.id}>다운로드</Button>
                      </a>
                    </span>
                  </Row>
                </Card.Body>
              </Card>
            </Col>
          </Row>
        </Col>
      </Row>
    </Container>
  );
}
