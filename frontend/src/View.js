import './View.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {useEffect, useState} from 'react';
import {Alert, Button, Card, Col, Container, Form, InputGroup, Row} from 'react-bootstrap';
import {getUrlPrefix, handleFetchErrors} from './Common';
import DirList from './DirList';

export default function View() {
  const [error, setError] = useState('');
  const [errorMessage, setErrorMessage] = useState('');

  const [treeData, setTreeData] = useState([]);
  const [entryId, setEntryId] = useState('');
  const [entryName, setEntryName] = useState('');
  const [size, setSize] = useState(0);
  const [encoding, setEncoding] = useState('');
  const [viewUrl, setViewUrl] = useState('');
  const [downloadUrl, setDownloadUrl] = useState('');

  const [dirListLoading, setDirListLoading] = useState(false);

  useEffect(() => {
    setDirListLoading(true);
    const someDirListUrl = getUrlPrefix() + "/somedirs";
    fetch(someDirListUrl)
      .then(handleFetchErrors)
      .then((response) => {
        response.json()
          .then((result) => {
            if (result['status'] === 'success') {
              setTreeData(result['result']);

              console.log("call /dirs for treeData");
              const fullDirListUrl = getUrlPrefix() + "/dirs";
              fetch(fullDirListUrl)
                .then(handleFetchErrors)
                .then((response) => {
                  response.json()
                    .then((result) => {
                      if (result['status'] === 'success') {
                        setTreeData(result['result']);
                      } else {
                        setErrorMessage(`directory list load failed, ${result['error']}`);
                      }
                    })
                    .catch((error) => {
                      setErrorMessage(`dir list load failed, ${error}`);
                    });
                })
                .catch((error) => {
                  setErrorMessage(`dir list load failed, ${error}`);
                });
            } else {
              setErrorMessage(`directory list load failed, ${result['error']}`);
            }
          })
          .catch((error) => {
            setErrorMessage(`dir list load failed, ${error}`);
          });
      })
      .catch((error) => {
        setErrorMessage(`dir list load failed, ${error}`);
      })
      .finally(() => {
        setDirListLoading(false);
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

    setDirListLoading(true);
    const fileContentUrl = getUrlPrefix() + '/dirs/' + encodeURIComponent(dirName) + '/files/' + encodeURIComponent(fileName);
    fetch(fileContentUrl)
      .then(handleFetchErrors)
      .then((response) => {
        response.json()
          .then((result) => {
            if (result.status === 'success') {
              setSize(result.result.size);
              setEncoding(result.result.encoding);
              setErrorMessage(null);
            } else {
              setErrorMessage(result.error);
            }
          })
          .catch((err) => {
            setError(err.message);
          });
      })
      .catch((error) => {
        setError(error.message);
      })
      .finally(() => {
        setDirListLoading(false);
      });
  }

  return (
    <Container id="view">
      <Row fluid="true">
        <Col md="3" lg="2" className="ps-0 pe-0">
          {
            dirListLoading && <div className="loading">로딩 중...</div>
          }
          {
            !dirListLoading && <DirList treeData={treeData} onClickHandler={fileClicked}/>
          }
        </Col>

        <Col md="9" lg="10">
          {
            error && <div>{error}</div>
          }
          <Row id="top_panel">
            <Col lg="12" className="ps-0 pe-0 me-0">
              <Card>
                {
                  errorMessage && <Alert variant="danger" className="mb-0">{errorMessage}</Alert>
                }
                <Card.Header>
                  파일 정보
                </Card.Header>
                <Card.Body>
                  <Row>
                    <Col xs="6">
                      <InputGroup>
                        <InputGroup.Text>파일명</InputGroup.Text>
                        <Form.Control defaultValue={entryName} readOnly/>
                      </InputGroup>
                    </Col>
                    <Col xs="3">
                      <InputGroup>
                        <InputGroup.Text>인코딩</InputGroup.Text>
                        <Form.Control defaultValue={encoding} readOnly/>
                      </InputGroup>
                    </Col>
                    <Col xs="3">
                      <InputGroup>
                        <InputGroup.Text>크기</InputGroup.Text>
                        <Form.Control defaultValue={size} readOnly/>
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
              </Card>
            </Col>
          </Row>
        </Col>
      </Row>
    </Container>
  );
}
