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

  const [topDirList, setToptopDirList] = useState('');
  const [entryId, setEntryId] = useState('');
  const [entryName, setEntryName] = useState('');

  const [author, setAuthor] = useState('');
  const [title, setTitle] = useState('');
  const [extension, setExtension] = useState('');
  const [size, setSize] = useState(0);
  const [encoding, setEncoding] = useState('');
  const [newFileName, setNewFileName] = useState('');

  const [fileMetadataLoading, setFileMetadataLoading] = useState(false);
  const [similarFileList, setSimilarFileList] = useState([]);
  const [similarFileListLoading, setSimilarFileListLoading] = useState(false);
  const [searchResult, setSearchResult] = useState('');
  const [searchResultLoading, setSearchResultLoading] = useState(false);
  const [selectedDirectory, setSelectedDirectory] = useState('');

  const [subTreeData, setSubTreeData] = useState([]);
  const [nextEntryIndex, setNextEntryIndex] = useState(0);
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
              setToptopDirList(result['result']);
              setErrorMessage(null);
            } else {
              setErrorMessage(result['error']);
            }
          })
          .catch((error) => {
            setErrorMessage(error.message);
          });
      })
      .catch((error) => {
        setErrorMessage(error.message);
      });

    return () => {
      // console.log('컴퍼넌트가 사라질 때 cleanup할 일을 여기서 처리해야 함');
      setToptopDirList(null);
      setEntryId('');
      setEntryName('');

      setSubTreeData(null);
      setNextEntryIndex(0);

      setAuthor('');
      setTitle('');
      setExtension('');
      setNewFileName('');
      setSize(0);
      setEncoding('');
    };
  }, [] /* rendered once */);

  useEffect(() => {
    if (author || title) {
      setNewFileName(`[${author}] ${title}.${extension}`);
    }
  }, [author, title]);

  const decomposeFileName = (fileName) => {
    console.log(`decomposeFileName(${fileName})`);
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
        //console.log(`pattern=${pattern}, match.groups=${JSON.stringify(match.groups)}`);
        setAuthor(author);
        setTitle(title);
        setExtension(extension);
        break;
      }
    }
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

    window.scrollTo(0, 0);

    // handling next file entry
    const entryId = key;
    const dirName = entryId.split('/')[0];
    const fileName = entryId.split('/')[1];
    setEntryId(entryId);
    setEntryName(fileName);
    setNextEntryIndex(nextIndex);
    setSubTreeData(subTreeData);
    setPropsData(props);
    setNewFileName(fileName);

    // decompose file name to (author, title, extension)
    decomposeFileName(fileName);

    // fetch file metadata (size, encoding)
    const fileMetadataUrl = getUrlPrefix() + '/dirs/' + encodeURIComponent(dirName) + '/files/' + encodeURIComponent(fileName);
    fetch(fileMetadataUrl)
      .then(handleFetchErrors)
      .then((response) => {
        response.json()
          .then((result) => {
            if (result.status === 'success') {
              setSize(result['result']['size']);
              setEncoding(result['result']['encoding']);
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
      });
  }, []);

  const authorChanged = useCallback((e, props) => {
    setAuthor(e.target.value);
  });

  const titleChanged = useCallback((e, props) => {
    setTitle(e.target.value);
  });

  const extensionChanged = useCallback((e, props) => {
    setExtension(e.target.value);
  });

  const newFileNameChanged = useCallback((e, props) => {
    setNewFileName(e.target.value);
  });

  const cutAuthorButtonClicked = useCallback((e, props) => {
    const tokens = author.split(' ');
    setAuthor(tokens[0]);
    setTitle(tokens[1]);
  }, [author]);

  const cutTitleButtonClicked = useCallback((e, props) => {
    const tokens = title.split(' ');
    setAuthor(tokens[0]);
    setTitle(tokens[1]);
  }, [title]);

  const exchangeButtonClicked = useCallback((e, props) => {
    console.log(`exchangeButtonClicked: author=${author}, title=${title}`);
    const newAuthor = title;
    const newTitle = author;
    setAuthor(newAuthor);
    setTitle(newTitle);
  }, [author, title]);

  const resetButtonClicked = useCallback((e, props) => {
    decomposeFileName(entryName);
  }, [entryName]);

  const changeClicked = useCallback(() => {
    alert('변경 버튼 클릭됨');
  }, []);

  const moveToUpperButtonClicked = useCallback(() => {
    console.log(`move to upper directory as '${newFileName}'`);
  }, [newFileName]);

  const selectDirectoryClicked = useCallback((e, props) => {
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
  }, [entryId]);

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
                  <Row>
                    <InputGroup>
                      <InputGroup.Text>저자</InputGroup.Text>
                      <Form.Control value={author} onChange={authorChanged}/>
                      <Button variant="outline-secondary" size="sm" onClick={cutAuthorButtonClicked}>
                        자르기
                        <FontAwesomeIcon icon={faCut}/>
                      </Button>
                      <Button variant="outline-secondary" size="sm" onClick={exchangeButtonClicked}>
                        교환
                        <FontAwesomeIcon icon={faRotate}/>
                      </Button>
                    </InputGroup>
                  </Row>
                  <Row>
                    <InputGroup>
                      <InputGroup.Text>제목</InputGroup.Text>
                      <Form.Control value={title} onChange={titleChanged}/>
                      <Button variant="outline-secondary" size="sm" onClick={cutTitleButtonClicked}>
                        자르기
                        <FontAwesomeIcon icon={faCut}/>
                      </Button>
                      <Button variant="outline-secondary" size="sm" onClick={resetButtonClicked}>
                        초기화
                        <FontAwesomeIcon icon={faClockRotateLeft}/>
                      </Button>
                    </InputGroup>
                  </Row>
                  <Row>
                    <Col xs="3">
                      <InputGroup>
                        <InputGroup.Text>확장자</InputGroup.Text>
                        <Form.Control value={extension} onChange={extensionChanged}/>
                      </InputGroup>
                    </Col>
                    <Col xs="9">
                      <InputGroup>
                        <InputGroup.Text>파일명</InputGroup.Text>
                        <Form.Control value={newFileName} onChange={newFileNameChanged}/>
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
                      <a href={`https://search.shopping.naver.com/book/search?bookTabType=ALL&pageIndex=1&pageSize=40&sort=REL&query=${author}+${title}`} target="_blank" rel="noreferrer">
                        <Button variant="outline-primary" size="sm">네이버 검색</Button>
                      </a>
                      <a href={`https://www.google.com/search?sourceid=chrome&ie=UTF-8&oq=${author}+${title}&q=${author}+${title}`} target="_blank" rel="noreferrer">
                        <Button variant="outline-primary" size="sm">구글 검색</Button>
                      </a>
                    </Col>
                  </Row>
                  <Row className="mt-2 button_group">
                    <Button variant="outline-warning" size="sm" onClick={moveToUpperButtonClicked} disabled={!newFileName}>
                      상위로
                      <FontAwesomeIcon icon={faUpload}/>
                    </Button>

                    {
                      topDirList && topDirList.filter(dir => dir.key !== entryId.split('/')[0])
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
                      <Form.Control value={selectedDirectory} readOnly/>
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
                  <a href={`/view/${entryId}`} target="_blank" rel="noreferrer">
                    <Button variant="outline-primary" size="sm" className="float-end">새 창에서 전체보기</Button>
                  </a>
                </Card.Header>
                <Card.Body>
                  <ViewSingle key={entryId} entryId={entryId} lineCount={100}/>
                </Card.Body>
              </Card>
            </Col>
          </Row>
        </Col>
      </Row>
    </Container>
  );
}
