import './View.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {Suspense, useEffect, useState} from 'react';
import {Alert, Button, Card, Col, Container, Form, InputGroup, Row} from 'react-bootstrap';
import {jsonGetReq, getUrlPrefix} from './Common';
import DirList from './DirList';

export default function View() {
  const [errorMessage, setErrorMessage] = useState('');

  const [treeData, setTreeData] = useState([]);
  const [entryId, setEntryId] = useState('');
  const [entryName, setEntryName] = useState('');
  const [size, setSize] = useState(0);
  const [encoding, setEncoding] = useState('');
  const [viewUrl, setViewUrl] = useState('');
  const [downloadUrl, setDownloadUrl] = useState('');

  useEffect(() => {
    console.log(`call /somedir for partial tree data`)
    const someDirListUrl = '/somedirs';
    jsonGetReq(someDirListUrl, (result) => {
      setTreeData(result['result']);

      console.log("call /dirs for full tree data");
      const fullDirListUrl = '/dirs';
      jsonGetReq(fullDirListUrl, (result) => {
        setTreeData(result);
      }, (error) => {
        setErrorMessage(`dir list load failed, ${error}`);
      })
    }, (error) => {
      setErrorMessage(`dir list load failed, ${error}`);
    });

    return () => {
      setTreeData([]);
      setEntryId('');
      setEntryName('');
      setSize(0);
      setEncoding('');
      setViewUrl('');
      setDownloadUrl('');
    }
  }, []);

  function fileClicked(key) {
    console.log(`fileClicked: key=${key}`);

    const dirName = key.split('/')[0];
    const fileName = key.split('/')[1];
    setEntryId(key);
    setEntryName(fileName);

    setViewUrl('/view/' + encodeURIComponent(dirName) + '/' + encodeURIComponent(fileName));
    setDownloadUrl(getUrlPrefix() + '/download/dirs/' + encodeURIComponent(dirName) + '/files/' + encodeURIComponent(fileName));

    const fileMetadataUrl = '/dirs/' + encodeURIComponent(dirName) + '/files/' + encodeURIComponent(fileName);
    jsonGetReq(fileMetadataUrl, (result) => {
      console.log(`fileMetadataUrl=${fileMetadataUrl}, result=`, result);
      setSize(result['size']);
      setEncoding(result['encoding']);
    }, (error) => {
      setErrorMessage(`file metadata load failed, ${error}`);
    })
  }

  return (
    <Container id="view">
      <Row fluid="true">
        <Col md="3" lg="2" className="ps-0 pe-0 section">
          <Suspense fallback={<div className="loading">로딩 중...</div>}>
            <DirList treeData={treeData} onClickHandler={fileClicked}/>
          </Suspense>
        </Col>

        <Col md="9" lg="10" className="section">
          <Row id="top_panel">
            <Col lg="12" className="ps-0 pe-0 me-0 ">
              <Card>
                {
                  errorMessage && <Alert variant="danger" className="mb-0">{errorMessage}</Alert>
                }
                <Card.Header>
                  파일 정보
                </Card.Header>
                {
                  !errorMessage &&
                  <Card.Body>
                    <Row>
                      <Col xs="6">
                        <InputGroup>
                          <InputGroup.Text>파일명</InputGroup.Text>
                          <Form.Control value={entryName} readOnly/>
                        </InputGroup>
                      </Col>
                      <Col xs="3">
                        <InputGroup>
                          <InputGroup.Text>인코딩</InputGroup.Text>
                          <Form.Control value={encoding} readOnly/>
                        </InputGroup>
                      </Col>
                      <Col xs="3">
                        <InputGroup>
                          <InputGroup.Text>크기</InputGroup.Text>
                          <Form.Control value={size} readOnly/>
                        </InputGroup>
                      </Col>
                    </Row>

                    <Row className="mt-1">
                      <span>
                        <a href={viewUrl} target="_blank" rel="noreferrer">
                          <Button variant="outline-secondary" size="sm" disabled={!entryId}>새 창에서 전체 보기</Button>
                        </a>
                        <a href={downloadUrl} target="_blank" rel="noreferrer">
                          <Button variant="outline-secondary" size="sm" disabled={!entryId}>다운로드</Button>
                        </a>
                      </span>
                    </Row>
                  </Card.Body>
                }
              </Card>
            </Col>
          </Row>
        </Col>
      </Row>
    </Container>
  );
}
