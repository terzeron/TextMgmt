import './Edit.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {useCallback, useEffect, useMemo, useRef, useState} from 'react';

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
  const [errorMessage, setErrorMessage] = useState('');

  const [dirList, setDirList] = useState([]);
  const [entryInfo, setEntryInfo] = useState({
    id: '',
    name: '',
  });
  const [bookInfo, setBookInfo] = useState({
    author: '',
    title: '',
    extension: '',
  });
  const [newFileName, setNewFileName] = useState(null);
  const [fileMetadata, setFileMetadata] = useState({
    size: '',
    encoding: '',
  });
  const [fileMetadataLoading, setFileMetadataLoading] = useState(false);
  const [similarFileList, setSimilarFileList] = useState([]);
  const [similarFileListLoading, setSimilarFileListLoading] = useState(false);
  const [searchResult, setSearchResult] = useState(null);
  const [searchResultLoading, setSearchResultLoading] = useState(false);
  const [selectedDirectory, setSelectedDirectory] = useState(null);
  const [subTreeData, setSubTreeData] = useState([]);
  const [nextEntryIndex, setNextEntryIndex] = useState(null);
  const [propsData, setPropsData] = useState({});

  useEffect(() => {
    console.log(`Edit: useEffect() `);

    console.log("call /topdirs for other dir list")
    fetch(`${getUrlPrefix()}/topdirs`)
      .then(handleFetchErrors)
      .then((response) => {
        response.json()
          .then((result) => {
            if (result.status === 'success') {
              const dirListData = result['result'];
              setDirList(dirListData);
              setFileMetadata(result['result']);
              setErrorMessage(null);
            } else {
              setErrorMessage(result['error']);
              setFileMetadata({
                size: '',
                encoding: '',
              });
            }
          })
          .catch((error) => {
            setErrorMessage(error.message);
            setFileMetadata({
              size: '',
              encoding: '',
            });
          });
      })
      .catch((error) => {
        setErrorMessage(error.message);
        setFileMetadata({
          size: '',
          encoding: '',
        });
      });

    return () => {
      // console.log('컴퍼넌트가 사라질 때 cleanup할 일을 여기서 처리해야 함');
    };
  }, [] /* rendered once */);

  useEffect(() => {
    if (bookInfo && (bookInfo.author || bookInfo.title)) {
      setNewFileName(`[${bookInfo.author}] ${bookInfo.title}.${bookInfo.extension}`);
    }
  }, [bookInfo]);

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
        setBookInfo({
          author,
          title,
          extension,
        });
        break;
      }
    }
  };

  const showfileMetadata = () => {
    if (fileMetadataLoading) return <div>로딩 중...</div>;
    return <div>{fileMetadata.content}</div>;
  };

  const showSimilarFileList = () => {
    if (similarFileListLoading) return <div>로딩 중...</div>;
    return <div>{similarFileList}</div>;
  };

  const showSearchResult = () => {
    if (searchResultLoading) return <div>로딩 중...</div>;
    return <div>{searchResult}</div>;
  };

  const fileClicked = useCallback((key, label, props, subTreeData, nextIndex) => {
    console.log(`fileClicked: key=${key}, label=${label}, props=${JSON.stringify(props)}, nextIndex=${nextIndex}`);
    const entryId = key;
    const dirName = entryId.split('/')[0];
    const fileName = entryId.split('/')[1];
    setEntryInfo({id: entryId, name: fileName});
    setNextEntryIndex(nextIndex);
    setSubTreeData(subTreeData);
    setPropsData(props);
    setNewFileName(fileName);
    decomposeFileName(fileName);

    window.scrollTo(0, 0);

    const fileMetadataUrl = getUrlPrefix() + '/dirs/' + encodeURIComponent(dirName) + '/files/' + encodeURIComponent(fileName);
    fetch(fileMetadataUrl)
      .then(handleFetchErrors)
      .then((response) => {
        response.json()
          .then((result) => {
            if (result.status === 'success') {
              setFileMetadata(result.result);
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
        setFileMetadata({
          size: '',
          encoding: '',
        });
      });
  }, []);

  const cutAuthorClicked = useCallback((e, props) => {
    e.preventDefault();
    const tokens = bookInfo.author.split(' ');
    setBookInfo({
      author: tokens[0],
      title: tokens[1],
      extension: bookInfo.extension,
    });
  }, [bookInfo]);

  const cutTitleClicked = useCallback((e, props) => {
    e.preventDefault();
    const tokens = bookInfo.title.split(' ');
    setBookInfo({
      author: tokens[0],
      title: tokens[1],
      extension: bookInfo.extension,
    });
  }, [bookInfo]);

  const exchangeAuthorAndTitleClicked = useCallback((e, props) => {
    e.preventDefault();
    const tokens = [bookInfo.author, bookInfo.title];
    setBookInfo({
      author: tokens[1],
      title: tokens[0],
      extension: bookInfo.extension,
    });
  }, [bookInfo]);

  const resetAuthorAndTitleClicked = useCallback((e, props) => {
    e.preventDefault();
    decomposeFileName(entryInfo.name);
  }, [entryInfo]);

  const changeClicked = useCallback(() => {
    alert('변경 버튼 클릭됨');
  }, []);

  const moveToUpperDirectoryClicked = useCallback(() => {
    console.log(`move to upper directory as '${newFileName}'`);
  }, [newFileName]);

  const selectDirectoryClicked = useCallback((e, props) => {
    e.preventDefault();
    setSelectedDirectory(props);
  }, []);

  const moveToDirectoryClicked = useCallback(() => {
    console.log(`move to '${selectedDirectory}' as '${newFileName}'`);
  }, [newFileName]);

  const removeClicked = useCallback(() => {
    alert('삭제 버튼 클릭됨');
  }, []);

  const toNextBookClicked = useCallback(() => {
    console.log('toNextBookClicked');
    if (subTreeData[nextEntryIndex]) {
      fileClicked(propsData['parent'] + '/' + subTreeData[nextEntryIndex].key, subTreeData[nextEntryIndex].label, propsData, subTreeData, nextEntryIndex + 1);
    } else {
      alert('마지막 파일입니다.');
    }
  }, [entryInfo]);

  if (loading) return <div>로딩 중..</div>;
  if (errorMessage) return <div>{errorMessage}</div>;
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
                        <Form.Control defaultValue={entryInfo.name} readOnly/>
                      </InputGroup>
                    </Col>
                    <Col xs="3">
                      <InputGroup>
                        <InputGroup.Text>인코딩</InputGroup.Text>
                        <Form.Control defaultValue={fileMetadata.encoding} readOnly/>
                      </InputGroup>
                    </Col>
                    <Col xs="3">
                      <InputGroup>
                        <InputGroup.Text>크기</InputGroup.Text>
                        <Form.Control defaultValue={fileMetadata.size} readOnly/>
                      </InputGroup>
                    </Col>
                  </Row>
                  <Row>
                    <InputGroup>
                      <InputGroup.Text>저자</InputGroup.Text>
                      <Form.Control defaultValue={bookInfo.author}/>
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
                      <Form.Control defaultValue={bookInfo.title}/>
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
                        <Form.Control defaultValue={bookInfo.extension}/>
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
                      <Button variant="outline-success" size="sm" onClick={toNextBookClicked}>다음 책으로</Button>
                      <a href={`https://search.shopping.naver.com/book/search?bookTabType=ALL&pageIndex=1&pageSize=40&sort=REL&query=${bookInfo.author}+${bookInfo.title}`} target="_blank" rel="noreferrer">
                        <Button variant="outline-primary" size="sm">네이버 검색</Button>
                      </a>
                      <a href={`https://www.google.com/search?sourceid=chrome&ie=UTF-8&oq=${bookInfo.author}+${bookInfo.title}&q=${bookInfo.author}+${bookInfo.title}`} target="_blank" rel="noreferrer">
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
                      dirList.filter(dir => dir.key !== entryInfo['id'].split('/')[0])
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
                  <a href={`/view/${entryInfo['id']}`} target="_blank" rel="noreferrer">
                    <Button variant="outline-primary" size="sm" className="float-end">새 창에서 전체보기</Button>
                  </a>
                </Card.Header>
                <Card.Body>
                  <ViewSingle key={entryInfo['id']} entryId={entryInfo['id']}/>
                </Card.Body>
              </Card>
            </Col>
          </Row>
        </Col>
      </Row>
    </Container>
  );
}
