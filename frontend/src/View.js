import './View.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {useEffect, useState} from 'react';
import {
  Button, Card, Col, Container, Form, InputGroup, Row,
} from 'react-bootstrap';
import { getUrlPrefix, handleFetchErrors } from './Common';
import DirList from './DirList';

export default function View() {
  const [loading, setLoading] = useState(false);
  const [errorMessage, setErrorMessage] = useState(null);
  const [viewUrl, setViewUrl] = useState(null);
  const [downloadUrl, setDownloadUrl] = useState('');
  const [entryInfo, setEntryInfo] = useState({
    id: '',
    name: '',
  });
  const [fileContent, setFileContent] = useState({
    size: '',
    encoding: '',
  });

  function fileClicked(key, label, props, subTreeData, nextIndex) {
    console.log(`fileClicked: key=${key}, label=${label}, props=${props}, nextIndex=${nextIndex}`);

    const entryId = key;
    const dirName = entryId.split('/')[0];
    const fileName = entryId.split('/')[1];

    setEntryInfo({ id: entryId, name: fileName });
    console.log('viewUrl=/view/' + encodeURIComponent(dirName) + '/' + encodeURIComponent(fileName));
    console.log('downloadUrl=' + getUrlPrefix() + '/download/dirs/' + encodeURIComponent(dirName) + '/files/' + encodeURIComponent(fileName));
    setViewUrl('/view/' + encodeURIComponent(dirName) + '/' + encodeURIComponent(fileName));
    setDownloadUrl(getUrlPrefix() + '/download/dirs/' + encodeURIComponent(dirName) + '/files/' + encodeURIComponent(fileName));

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
              });
            }
          })
          .catch((err) => {
            setErrorMessage(err.message);
            setFileContent({
              size: '',
              encoding: '',
            });
          });
      })
      .catch((error) => {
        setErrorMessage(error.message);
        setFileContent({
          size: '',
          encoding: '',
        });
      });
  }

  if (loading) return <div>로딩 중..</div>;
  if (errorMessage) return <div>{errorMessage}</div>;
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
                        <Form.Control defaultValue={entryInfo.name} readOnly />
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
                      <a href={viewUrl} target="_blank" rel="noreferrer">
                        <Button variant="outline-secondary" size="sm" disabled={!entryInfo.id}>새 창에서 전체 보기</Button>
                      </a>
                      <a href={downloadUrl} target="_blank" rel="noreferrer">
                        <Button variant="outline-secondary" size="sm" disabled={!entryInfo.id}>다운로드</Button>
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
