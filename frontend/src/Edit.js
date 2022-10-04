import './Edit.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {useEffect, useState} from 'react';

import {
  Container, Row, Col, Card, Form, Button, InputGroup, Alert,
} from 'react-bootstrap';
import {FontAwesomeIcon} from '@fortawesome/react-fontawesome';
import {
  faCheck, faClockRotateLeft, faCut, faRotate, faTrash, faTruckMoving, faUpload,
} from '@fortawesome/free-solid-svg-icons';
import DirList from './DirList';
import {getUrlPrefix, handleFetchErrors} from './Common';
import ViewSingle from "./ViewSingle";

export default function Edit() {
  const [loading, setLoading] = useState(false);
  const [errorMessage, setErrorMessage] = useState("");

  const [dirList, setDirList] = useState([]);
  const [fileMetadata, setFileMetadata] = useState({
    id: '',
    name: '',
  });
  const [fileInfo, setFileInfo] = useState({
    author: '',
    title: '',
    extension: '',
  });
  const [newFileName, setNewFileName] = useState(null);
  const [fileContent, setFileContent] = useState({
    size: '',
    encoding: '',
    content: '',
  });
  const [fileContentLoading, setFileContentLoading] = useState(false);
  const [similarFileList, setSimilarFileList] = useState([]);
  const [similarFileListLoading, setSimilarFileListLoading] = useState(false);
  const [searchResult, setSearchResult] = useState(null);
  const [searchResultLoading, setSearchResultLoading] = useState(false);
  const [selectedDirectory, setSelectedDirectory] = useState(null);

  useEffect(() => {
    fetch(`${getUrlPrefix()}/topdirs`)
      .then(handleFetchErrors)
      .then((response) => {
        response.json()
          .then((result) => {
            if (result.status === 'success') {
              const dirListData = result['result'];
              setDirList(dirListData);
              setFileContent(result['result']);
              setErrorMessage(null);
            } else {
              setErrorMessage(result['error']);
              setFileContent({
                size: '',
                encoding: '',
                content: '',
              });
            }
          })
          .catch((error) => {
            setErrorMessage(error.message);
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

    return () => {
      // console.log('컴퍼넌트가 사라질 때 cleanup할 일을 여기서 처리해야 함');
    };
  }, [] /* rendered once */);

  useEffect(() => {
    if (fileInfo && (fileInfo.author || fileInfo.title)) {
      setNewFileName(`[${fileInfo.author}] ${fileInfo.title}.${fileInfo.extension}`);
    }
  }, [fileInfo]);

  const decomposeFileName = (fileName) => {
    // console.log(`decomposeFileName(${fileName})`);
    // [ 저자 ] 제목 . 확장자
    const pattern1 = /^\s*\[(?<author>.*?)]\s*(?<title>.*?)\s*\.(?<extension>txt|epub|zip|pdf)\s*$/;
    // 제목 [ 저자 ] . 확장자
    const pattern2 = /^\s*(?<title>.*?)\s*\[\s*(?<author>.*?)\s*]\s*\.(?<extension>txt|epub|zip|pdf)\s*$/;
    // 제목 @ 저자 . 확장자
    const pattern3 = /^\s*(?<title>.*?)\s*@\s*(?<author>.*?)\s*\.(?<extension>txt|epub|zip|pdf)\s*$/;
    // 저자 - 제목 . 확장자 or 저자 _ 제목 . 확장자
    const pattern4 = /^\s*(?<author>.*?)\s*[_-]\s*(?<title>.*?)\s*\.(?<extension>txt|epub|zip|pdf)\s*$/;
    // 제목
    const pattern5 = /^\s*(?<title>.*?)\s*\.(?<extension>txt|epub|zip|pdf)\s*$/;
    let author = '';
    let title = '';
    let extension = '';

    for (const pattern of [pattern1, pattern2, pattern3, pattern4, pattern5]) {
      const match = pattern.exec(fileName);
      if (match) {
        author = match.groups.author || '';
        title = match.groups.title || '';
        extension = match.groups.extension || '';
        setFileInfo({
          author,
          title,
          extension,
        });
        break;
      }
    }
  };

  const showFileContent = () => {
    if (fileContentLoading) return <div>로딩 중...</div>;
    return <div>{fileContent.content}</div>;
  };

  const showSimilarFileList = () => {
    if (similarFileListLoading) return <div>로딩 중...</div>;
    return <div>{similarFileList}</div>;
  };

  const showSearchResult = () => {
    if (searchResultLoading) return <div>로딩 중...</div>;
    return <div>{searchResult}</div>;
  };

  function fileClicked(key, label, props) {
    const fileId = key;
    const dirName = fileId.split('/')[0];
    const fileName = fileId.split('/')[1];
    setFileMetadata({id: fileId, name: fileName});
    setNewFileName(fileName);
    decomposeFileName(fileName);

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
            }
          })
          .catch((error) => {
            setErrorMessage(error.message);
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
  };

  const cutAuthorClicked = (e, props) => {
    e.preventDefault();
    const tokens = fileInfo.author.split(' ');
    setFileInfo({
      author: tokens[0],
      title: tokens[1],
      extension: fileInfo.extension,
    });
  };

  const cutTitleClicked = (e, props) => {
    e.preventDefault();
    const tokens = fileInfo.title.split(' ');
    setFileInfo({
      author: tokens[0],
      title: tokens[1],
      extension: fileInfo.extension,
    });
  };

  const exchangeAuthorAndTitleClicked = (e, props) => {
    e.preventDefault();
    const tokens = [fileInfo.author, fileInfo.title];
    setFileInfo({
      author: tokens[1],
      title: tokens[0],
      extension: fileInfo.extension,
    });
  };

  const resetAuthorAndTitleClicked = (e, props) => {
    e.preventDefault();
    decomposeFileName(fileMetadata.name);
  };

  const changeClicked = () => {
    alert('변경 버튼 클릭됨');
  };

  const moveToUpperDirectoryClicked = () => {
    console.log(`move to upper directory as '${newFileName}'`);
  };

  const selectDirectoryClicked = (e, props) => {
    e.preventDefault();
    setSelectedDirectory(props);
  };

  const moveToDirectoryClicked = () => {
    console.log(`move to '${selectedDirectory}' as '${newFileName}'`);
  };

  const removeClicked = () => {
    alert('삭제 버튼 클릭됨');
  };

  if (loading) return <div>로딩 중..</div>;
  return (
    <Container id="edit">
      <Row fluid="true">
        <Col md="3" lg="2" className="ps-0 pe-0">
          <DirList onClickHandler={fileClicked}/>
        </Col>

        <Col md="9" lg="10">
          <Row id="top_panel">
            <Col id="left_panel" lg="5" className="ps-0 pe-0">
              <Card>
                <Card.Header>
                  파일 정보
                </Card.Header>
                <Card.Body>
                  <Row>
                    <Col xs="6">
                      <InputGroup>
                        <InputGroup.Text>파일명</InputGroup.Text>
                        <Form.Control defaultValue={fileMetadata.name} readOnly/>
                      </InputGroup>
                    </Col>
                    <Col xs="3">
                      <InputGroup>
                        <InputGroup.Text>인코딩</InputGroup.Text>
                        <Form.Control defaultValue={fileContent.encoding} readOnly/>
                      </InputGroup>
                    </Col>
                    <Col xs="3">
                      <InputGroup>
                        <InputGroup.Text>크기</InputGroup.Text>
                        <Form.Control defaultValue={fileContent.size} readOnly/>
                      </InputGroup>
                    </Col>
                  </Row>
                  <Row>
                    <InputGroup>
                      <InputGroup.Text>저자</InputGroup.Text>
                      <Form.Control defaultValue={fileInfo.author}/>
                      <Button variant="outline-secondary" size="sm" onClick={cutAuthorClicked}>
                        자르기
                        <FontAwesomeIcon icon={faCut}/>
                      </Button>
                      <Button variant="outline-secondary" size="sm" onClick={exchangeAuthorAndTitleClicked}>
                        교환
                        <FontAwesomeIcon icon={faRotate}/>
                      </Button>
                    </InputGroup>
                  </Row>
                  <Row>
                    <InputGroup>
                      <InputGroup.Text>제목</InputGroup.Text>
                      <Form.Control defaultValue={fileInfo.title}/>
                      <Button variant="outline-secondary" size="sm" onClick={cutTitleClicked}>
                        자르기
                        <FontAwesomeIcon icon={faCut}/>
                      </Button>
                      <Button variant="outline-secondary" size="sm" onClick={resetAuthorAndTitleClicked}>
                        초기화
                        <FontAwesomeIcon icon={faClockRotateLeft}/>
                      </Button>
                    </InputGroup>
                  </Row>
                  <Row>
                    <Col xs="3">
                      <InputGroup>
                        <InputGroup.Text>확장자</InputGroup.Text>
                        <Form.Control defaultValue={fileInfo.extension}/>
                      </InputGroup>
                    </Col>
                    <Col xs="9">
                      <InputGroup>
                        <InputGroup.Text>파일명</InputGroup.Text>
                        <Form.Control defaultValue={newFileName}/>
                        <Button variant="outline-success" size="sm" onClick={changeClicked}>
                          변경
                          <FontAwesomeIcon icon={faCheck}/>
                        </Button>
                        <Button variant="outline-danger" size="sm" onClick={removeClicked}>
                          삭제
                          <FontAwesomeIcon icon={faTrash}/>
                        </Button>
                      </InputGroup>
                    </Col>
                  </Row>
                  <Row className="mt-2 button_group">
                    <Col>
                      <a href={`https://search.shopping.naver.com/book/search?bookTabType=ALL&pageIndex=1&pageSize=40&sort=REL&query=${fileInfo.author}+${fileInfo.title}`} target="_blank" rel="noreferrer">
                        <Button variant="outline-primary" size="sm">네이버 검색</Button>
                      </a>
                      <a href={`https://www.google.com/search?sourceid=chrome&ie=UTF-8&oq=${fileInfo.author}+${fileInfo.title}&q=${fileInfo.author}+${fileInfo.title}`} target="_blank" rel="noreferrer">
                        <Button variant="outline-primary" size="sm">구글 검색</Button>
                      </a>
                    </Col>
                  </Row>
                  <Row className="mt-2 button_group">
                    <Button variant="outline-warning" size="sm" onClick={moveToUpperDirectoryClicked} disabled={!newFileName}>
                      상위로
                      <FontAwesomeIcon icon={faUpload}/>
                    </Button>

                    {
                      dirList.filter(dir => dir.key !== fileMetadata['id'].split('/')[0])
                        .map(dir =>
                          <Button
                            variant="outline-secondary"
                            size="sm"
                            key={dir.key}
                            onClick={(e) => {
                              selectDirectoryClicked(e, dir.key);
                            }}>
                            {`${dir.key}`}
                          </Button>
                        )
                    }
                  </Row>

                  <Row className="mt-2">
                    <InputGroup className="ms-0 me-0">
                      <Form.Control defaultValue={selectedDirectory} readOnly/>
                      <Button variant="outline-warning" size="sm" onClick={moveToDirectoryClicked} disabled={!newFileName || !selectedDirectory}>
                        로 옮기기
                        <FontAwesomeIcon icon={faTruckMoving}/>
                      </Button>
                    </InputGroup>
                  </Row>
                </Card.Body>
              </Card>
              <Card>
                <Card.Header>
                  실행 결과
                </Card.Header>
                <Card.Body>
                  {
                    errorMessage && <Alert variant="danger" className="mb-0">{errorMessage}</Alert>
                  }
                </Card.Body>
              </Card>
            </Col>

            <Col id="right_panel" lg="7" className="ps-0 pe-0">
              <Card>
                <Card.Header>
                  유사한 파일 목록
                </Card.Header>
                <Card.Body>
                  {showSimilarFileList()}
                </Card.Body>
              </Card>
              <Card>
                <Card.Header>
                  검색 결과
                </Card.Header>
                <Card.Body>
                  {showSearchResult()}
                </Card.Body>
              </Card>
            </Col>
          </Row>

          <Row id="bottom_panel">
            <Col id="right_panel" className="ps-0 pe-0">
              <Card>
                <Card.Header>
                  파일 보기
                  <a href={`/view_single/${fileMetadata['id']}`} target="_blank" rel="noreferrer">
                    <Button variant="outline-primary" size="sm" className="float-end">새 창에서 전체보기</Button>
                  </a>
                </Card.Header>
                <Card.Body>
                  {
                    fileMetadata['id'] && <ViewSingle fileId={fileMetadata['id']}/>
                  }
                </Card.Body>
              </Card>
            </Col>
          </Row>
        </Col>
      </Row>
    </Container>
  );
}
