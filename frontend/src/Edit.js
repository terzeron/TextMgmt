import './Edit.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {useCallback, useEffect, useState} from 'react';

import {Alert, Button, Card, Col, Container, Form, InputGroup, Row} from 'react-bootstrap';
import {FontAwesomeIcon} from '@fortawesome/react-fontawesome';
import {faCheck, faClockRotateLeft, faCut, faRotate, faTrash, faTruckMoving, faUpload} from '@fortawesome/free-solid-svg-icons';
import DirList from './DirList';
import {getRandomDarkColor, getUrlPrefix, handleFetchErrors} from './Common';
import ViewSingle from "./ViewSingle";

export default function Edit() {
  const [error, setError] = useState('');
  const [errorMessage, setErrorMessage] = useState('');
  const [successMessage, setSuccessMessage] = useState('');

  const [treeData, setTreeData] = useState([]);
  const [topDirList, setToptopDirList] = useState('');
  const [entryId, setEntryId] = useState('');
  const [entryName, setEntryName] = useState('');

  const [author, setAuthor] = useState('');
  const [title, setTitle] = useState('');
  const [extension, setExtension] = useState('');
  const [size, setSize] = useState(0);
  const [encoding, setEncoding] = useState('');
  const [newFileName, setNewFileName] = useState('');

  const [dirListLoading, setDirListLoading] = useState(false);
  const [fileMetadataLoading, setFileMetadataLoading] = useState(false);
  const [similarFileList, setSimilarFileList] = useState([]);
  const [similarFileListLoading, setSimilarFileListLoading] = useState(false);
  const [searchResult, setSearchResult] = useState('');
  const [searchResultLoading, setSearchResultLoading] = useState(false);
  const [selectedDirectory, setSelectedDirectory] = useState('');

  const [nextEntryId, setNextEntryId] = useState('');

  useEffect(() => {
    console.log(`Edit: useEffect() `);

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
          })
      })
      .catch((error) => {
        setErrorMessage(`dir list load failed, ${error}`);
      })
      .finally(() => {
        setDirListLoading(false);
      });

    console.log("call /topdirs for other dir list")
    const topDirListUrl = getUrlPrefix() + '/topdirs';
    fetch(topDirListUrl)
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
      setTreeData(null);

      setToptopDirList(null);
      setEntryId('');
      setEntryName('');

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
    setAuthor('');
    setTitle('');
    setExtension('');

    // [ 저자 ] 제목 . 확장자
    const pattern1 = /^\s*\[(?<author>.*?)]\s*(?<title>.*?)\s*\.(?<extension>txt|epub|zip|pdf)\s*$/;
    // ( 저자 ) 제목 . 확장자
    const pattern2 = /^\s*\((?<author>.*?)\)\s*(?<title>.*?)\s*\.(?<extension>txt|epub|zip|pdf)\s*$/;
    // 제목 [ 저자 ] . 확장자
    const pattern3 = /^\s*(?<title>.*?)\s*\[\s*(?<author>.*?)\s*]\s*\.(?<extension>txt|epub|zip|pdf)\s*$/;
    // 제목 @ 저자 . 확장자
    const pattern4 = /^\s*(?<title>.*?)\s*@\s*(?<author>.*?)\s*\.(?<extension>txt|epub|zip|pdf)\s*$/;
    // 저자 - 제목 . 확장자 or 저자 _ 제목 . 확장자
    const pattern5 = /^\s*(?<author>.*?)\s*[_-]\s*(?<title>.*?)\s*\.(?<extension>txt|epub|zip|pdf)\s*$/;
    // 제목
    let author = '';
    let title = '';
    let extension = '';

    let isChanged = false;
    for (const pattern of [pattern1, pattern2, pattern3, pattern4, pattern5]) {
      const match = pattern.exec(fileName);
      if (match) {
        author = match.groups.author || '';
        title = match.groups.title || '';
        extension = match.groups.extension || '';
        setAuthor(author);
        setTitle(title);
        setExtension(extension);
        isChanged = true;
        break;
      }
    }
    if (isChanged === false) {
      const finalPattern = /^(?<title>.+)\.(?<extension>txt|epub|zip|pdf)\s*$/;
      const match = finalPattern.exec(fileName);
      if (match) {
        title = match.groups.title || '';
        extension = match.groups.extension || '';
        setTitle(title);
        setExtension(extension);
      }
    }
  };

  const fileClicked = useCallback((key) => {
    console.log(`fileClicked: key=${key}`);

    window.scrollTo(0, 0);

    const dirName = key.split('/')[0];
    const fileName = key.split('/')[1];
    setEntryId(key);
    setEntryName(fileName);
    setNewFileName(fileName);

    // reset
    setSuccessMessage(null);

    // determine nextEntryId
    //for (let i = 0; i < treeData.length; i++) {
    for (let treeDataItem of treeData) {
      if (treeDataItem.key === dirName) {
        const indexClicked = treeDataItem.nodes.findIndex((item) => item.key === fileName);
        if (indexClicked < treeDataItem.nodes.length - 1) {
          const nextEntryId = treeDataItem.nodes[indexClicked + 1].key;
          setNextEntryId(dirName + '/' + nextEntryId);
        } else {
          setNextEntryId(null);
        }
        break;
      }
    }

    // decompose file name to (author, title, extension)
    decomposeFileName(fileName);

    // fetch file metadata (size, encoding)
    setFileMetadataLoading(true);
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
            setError(error.message);
          });
      })
      .catch((error) => {
        setError(error.message);
      })
      .finally(() => {
        setFileMetadataLoading(false);
      });
  }, [treeData]);

  const authorChanged = useCallback((e) => {
    setAuthor(e.target.value);
  }, []);

  const titleChanged = useCallback((e) => {
    setTitle(e.target.value);
  }, []);

  const extensionChanged = useCallback((e) => {
    setExtension(e.target.value);
  }, []);

  const newFileNameChanged = useCallback((e) => {
    setNewFileName(e.target.value);
  }, []);

  const cutAuthorButtonClicked = useCallback(() => {
    const tokens = author.split(' ');
    setAuthor(tokens[0]);
    setTitle(tokens[1]);
  }, [author]);

  const cutTitleButtonClicked = useCallback(() => {
    const tokens = title.split(' ');
    setAuthor(tokens[0]);
    setTitle(tokens[1]);
  }, [title]);

  const exchangeButtonClicked = useCallback(() => {
    console.log(`exchangeButtonClicked: author=${author}, title=${title}`);
    const newAuthor = title;
    const newTitle = author;
    setAuthor(newAuthor);
    setTitle(newTitle);
  }, [author, title]);

  const resetButtonClicked = useCallback(() => {
    decomposeFileName(entryName);
  }, [entryName]);

  const changeButtonClicked = useCallback(() => {
    console.log(`changeButtonClicked: entryId=${entryId}, newFileName=${newFileName}`);
    const dirName = entryId.split('/')[0];
    const fileName = entryId.split('/')[1];
    const renameUrl = getUrlPrefix() + '/dirs/' + encodeURIComponent(dirName) + '/files/' + encodeURIComponent(fileName) + '/new/' + encodeURIComponent(newFileName);
    console.log(renameUrl);
    fetch(renameUrl, {method: 'PUT'})
      .then(handleFetchErrors)
      .then((response) => {
        response.json()
          .then((result) => {
            if (result.status === 'success') {
              setErrorMessage(null);
              setSuccessMessage("파일 이름이 변경되었습니다.");

              const newTreeData = [...treeData];
              let isChanged = false;

              for (let treeDataItem of newTreeData) {
                if (treeDataItem.key === dirName) {
                  for (let treeDataItemNode of treeDataItem.nodes) {
                    if (treeDataItemNode.key === fileName) {
                      treeDataItemNode.key = newFileName;
                      treeDataItemNode.title = newFileName;
                      isChanged = true;
                      break;
                    }
                  }
                }
                if (isChanged) {
                  break;
                }
              }
              setTreeData(newTreeData);
            } else {
              setErrorMessage(result['error']);
              setSuccessMessage(null);
            }
          })
          .catch((error) => {
            setErrorMessage(error.message);
          });
      })
      .catch((error) => {
        setErrorMessage(error.message);
      });
  }, [entryId, newFileName]);

  const moveToUpperButtonClicked = useCallback(() => {
    console.log(`move to upper directory as '${newFileName}'`);
  }, [newFileName]);

  const selectDirectoryClicked = useCallback((e, props) => {
    setSelectedDirectory(props);
  }, []);

  const moveToDirectoryClicked = useCallback(() => {
    console.log(`move to '${selectedDirectory}' as '${newFileName}'`);
  }, [newFileName, selectedDirectory]);

  const deleteButtonClicked = useCallback(() => {
    console.log(`deleteButtonClicked: entryId=${entryId}`);
    const dirName = entryId.split('/')[0];
    const fileName = entryId.split('/')[1];
    const deleteUrl = getUrlPrefix() + '/dirs/' + encodeURIComponent(dirName) + '/files/' + encodeURIComponent(fileName);
    console.log(deleteUrl);
    fetch(deleteUrl, {method: 'DELETE'})
      .then(handleFetchErrors)
      .then((response) => {
        response.json()
          .then((result) => {
            if (result.status === 'success') {
              setErrorMessage(null);
              setSuccessMessage("파일이 삭제되었습니다.");

              const newTreeData = [...treeData];
              for (let treeDataItem of newTreeData) {
                if (treeDataItem.key === dirName) {
                  const indexToDelete = treeDataItem.nodes.findIndex((item) => item.key === fileName);
                  if (indexToDelete > -1) {
                    treeDataItem.nodes.splice(indexToDelete, 1);
                    break;
                  }
                }
              }
              setTreeData(newTreeData);
            } else {
              setErrorMessage(result['error']);
              setSuccessMessage(null);
            }
          })
          .catch((error) => {
            setErrorMessage(error.message);
          });
      })
      .catch((error) => {
        setErrorMessage(error.message);
      });

  }, [entryId]);

  const toNextBookClicked = useCallback(() => {
    console.log(`toNextBookClicked: nextEntryId=${nextEntryId}`);
    if (nextEntryId) {
      fileClicked(nextEntryId);
    } else {
      alert('마지막 파일입니다.');
    }
  }, [nextEntryId]);

  return (
    <Container id="edit">
      <Row fluid="true">
        <Col md="3" lg="2" className="ps-0 pe-0">
          {
            dirListLoading && <div className="loading">로딩 중...</div>
          }
          <DirList treeData={treeData} onClickHandler={fileClicked}/>
        </Col>

        <Col md="9" lg="10">
          {
            error && <div>{error}</div>
          }
          <Row id="top_panel">
            <Col id="left_panel" lg="5" className="ps-0 pe-0">
              <Card>
                <Card.Header>
                  파일 정보
                </Card.Header>
                <Card.Body>
                  <Row>
                    {
                      fileMetadataLoading && <div className="loading">로딩 중...</div>
                    }
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
                        <Button variant="outline-success" size="sm" onClick={changeButtonClicked}>
                          변경
                          <FontAwesomeIcon icon={faCheck}/>
                        </Button>
                        <Button variant="outline-danger" size="sm" onClick={deleteButtonClicked}>
                          삭제
                          <FontAwesomeIcon icon={faTrash}/>
                        </Button>
                      </InputGroup>
                    </Col>
                  </Row>
                  <Row className="mt-2 button_group">
                    <Col>
                      <Button variant="outline-success" size="sm" onClick={toNextBookClicked}>다음 책으로</Button>
                      <a href={`https://www.yes24.com/Product/Search?domain=ALL&query=${encodeURIComponent(author)}+${encodeURIComponent(title)}`} target="_blank" rel="noreferrer">
                        <Button variant="outline-primary" size="sm">Yes24</Button>
                      </a>
                      <a href={`https://www.google.com/search?sourceid=chrome&ie=UTF-8&oq=${encodeURIComponent(author)}+${encodeURIComponent(title)}&q=${encodeURIComponent(author)}+${encodeURIComponent(title)}`} target="_blank" rel="noreferrer">
                        <Button variant="outline-primary" size="sm">구글</Button>
                      </a>
                      <a href={`https://search.shopping.naver.com/book/search?bookTabType=ALL&pageIndex=1&pageSize=40&sort=REL&query=${encodeURIComponent(author)}+${encodeURIComponent(title)}`} target="_blank" rel="noreferrer">
                        <Button variant="outline-primary" size="sm">네이버쇼핑</Button>
                      </a>
                      <a href={`https://series.naver.com/search/search.series?t=all&fs=novel&q=${encodeURIComponent(author)}+${encodeURIComponent(title)}`} target="_blank" rel="noreferrer">
                        <Button variant="outline-primary" size="sm">네이버시리즈</Button>
                      </a>
                      <a href={`https://novel.munpia.com/page/hd.platinum/view/search/keyword/${encodeURIComponent(author)}+${encodeURIComponent(title)}/order/search_result`} target="_blank" rel="noreferrer">
                        <Button variant="outline-primary" size="sm">문피아</Button>
                      </a>
                      <a href={`https://ridibooks.com/search?adult_exclude=n&q=${encodeURIComponent(author)}+${encodeURIComponent(title)}`} target="_blank" rel="noreferrer">
                        <Button variant="outline-primary" size="sm">RIDI</Button>
                      </a>
                    </Col>
                  </Row>
                  <Row className="mt-2 button_group">
                    <Button variant="outline-warning" size="sm" onClick={moveToUpperButtonClicked} disabled={!newFileName}>
                      상위로
                      <FontAwesomeIcon icon={faUpload}/>
                    </Button>
                    {
                      topDirList && topDirList
                        .filter(dir => dir.key !== entryId.split('/')[0])
                        .map((dir) => {
                            const category = dir.key.split('_')[0];
                            const subCategory = dir.key.split('_')[1];
                            const hasSubCategory = dir.key.includes('_');
                            if (hasSubCategory)
                              return (
                                <Button
                                  variant="outline-secondary"
                                  size="sm"
                                  key={dir.key}
                                  style={{
                                    backgroundColor: getRandomDarkColor(category),
                                    color: 'white'
                                  }}
                                  onClick={(e) => {
                                    selectDirectoryClicked(e, dir.key);
                                  }}>
                                  {subCategory}
                                </Button>
                              )
                            else
                              return (
                                <Button
                                  variant="outline-secondary"
                                  size="sm"
                                  key={dir.key}
                                  className="btn-light"
                                  onClick={(e) => {
                                    selectDirectoryClicked(e, dir.key);
                                  }}>
                                  {dir.key}
                                </Button>
                              )
                          }
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
                  {
                    successMessage && <Alert variant="success" className="mb-0">{successMessage}</Alert>
                  }
                </Card.Body>
              </Card>
            </Col>

            <Col id="right_panel" lg="7" className="ps-0 pe-0">
              <Card>
                {
                  similarFileListLoading && <div className="loading">로딩 중...</div>
                }
                <Card.Header>
                  유사한 파일 목록
                </Card.Header>
                <Card.Body>
                  {
                    similarFileList && similarFileList.map((item, index) => {
                      return (
                        {item}, {index}
                      )
                    })
                  }
                </Card.Body>
              </Card>

              <Card>
                {
                  searchResultLoading && <div className="loading">로딩 중...</div>
                }
                <Card.Header>
                  검색 결과
                </Card.Header>
                <Card.Body>
                  {
                    searchResult && searchResult.map((item, index) => {
                      return (
                        {item}, {index}
                      )
                    })
                  }
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
