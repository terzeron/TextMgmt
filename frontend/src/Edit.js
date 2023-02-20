import './Edit.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {useCallback, useEffect, useState, Suspense} from 'react';

import {Alert, Button, Card, Col, Container, Form, InputGroup, Row} from 'react-bootstrap';
import {FontAwesomeIcon} from '@fortawesome/react-fontawesome';
import {faCheck, faClockRotateLeft, faCut, faRotate, faTrash, faTruckMoving, faUpload} from '@fortawesome/free-solid-svg-icons';
import DirList from './DirList';
import {getRandomMediumColor, getUrlPrefix, jsonDeleteReq, jsonGetReq, jsonPutReq} from './Common';
import ViewSingle from "./ViewSingle";

export default function Edit() {
  const [errorMessage, setErrorMessage] = useState('');
  const [successMessage, setSuccessMessage] = useState('');

  const [treeData, setTreeData] = useState([]);
  const [topDirList, setToptopDirList] = useState('');
  const [entryId, setEntryId] = useState('');
  const [dirName, setDirName] = useState('');
  const [entryName, setEntryName] = useState('');

  const [author, setAuthor] = useState('');
  const [title, setTitle] = useState('');
  const [extension, setExtension] = useState('');
  const [size, setSize] = useState(0);
  const [encoding, setEncoding] = useState('');
  const [newFileName, setNewFileName] = useState('');
  const [viewUrl, setViewUrl] = useState('');
  const [downloadUrl, setDownloadUrl] = useState('');

  const [selectedDirectory, setSelectedDirectory] = useState('');
  const ROOT_DIRECTORY = '$$rootdir$$';
  const [nextEntryId, setNextEntryId] = useState('');

  useEffect(() => {
    console.log(`Edit: useEffect() `);

    console.log("call /dirs for treeData");
    const fullDirListUrl = '/dirs';
    jsonGetReq(fullDirListUrl, (result) => {
      setTreeData(result);
    }, (error) => {
      setErrorMessage(`directory list load failed, ${error}`);
    });

    console.log("call /topdirs for other dir list")
    const topDirListUrl = '/topdirs';
    jsonGetReq(topDirListUrl, (result) => {
      setToptopDirList(result);
    }, (error) => {
      setErrorMessage(`top dir list load failed, ${error}`);
    });

    return () => {
      setTreeData(null);

      setToptopDirList(null);
      setEntryId('');
      setDirName('');
      setEntryName('');

      setAuthor('');
      setTitle('');
      setExtension('');
      setNewFileName('');
      setSize(0);
      setEncoding('');
      setViewUrl('');
      setDownloadUrl('');
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
    const pattern1 = /^\s*\[(?<author>.*?)]\s*(?<title>.*?)\s*\.(?<extension>txt|epub|zip|pdf|html|hwp|dvju)\s*$/;
    // ( 저자 ) 제목 . 확장자
    const pattern2 = /^\s*\((?<author>.*?)\)\s*(?<title>.*?)\s*\.(?<extension>txt|epub|zip|pdf|html|hwp|dvju)\s*$/;
    // 제목 [ 저자 ] . 확장자
    const pattern3 = /^\s*(?<title>.*?)\s*\[\s*(?<author>.*?)\s*]\s*\.(?<extension>txt|epub|zip|pdf|html|hwp|dvju)\s*$/;
    // 제목 @ 저자 . 확장자
    const pattern4 = /^\s*(?<title>.*?)\s*@\s*(?<author>.*?)\s*\.(?<extension>txt|epub|zip|pdf|html|hwp|dvju)\s*$/;
    // 저자 - 제목 . 확장자 or 저자 _ 제목 . 확장자
    const pattern5 = /^\s*(?<author>.*?)\s*[_-]\s*(?<title>.*?)\s*\.(?<extension>txt|epub|zip|pdf|html|hwp|dvju)\s*$/;
    // 제목 ( 저자 )
    const pattern6 = /^\s*(?<title>.*?)\s*\(\s*(?<author>.*?)\s*\)\s*\.(?<extension>txt|epub|zip|pdf|html|hwp|dvju)\s*$/;
    // 모두 제목으로 간주
    const finalPattern = /^(?<title>.+)\.(?<extension>txt|epub|zip|pdf|html)\s*$/;

    let author = '';
    let title = '';
    let extension = '';

    let isChanged = false;
    for (const pattern of [pattern1, pattern2, pattern3, pattern4, pattern5, pattern6]) {
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
      const match = finalPattern.exec(fileName);
      if (match) {
        title = match.groups.title || '';
        extension = match.groups.extension || '';
        setTitle(title);
        setExtension(extension);
      }
    }
  };

  const determineNextEntryId = (treeData, dirName, fileName) => {
    let indexClicked = null;
    if (dirName === ROOT_DIRECTORY) {
      indexClicked = treeData.findIndex((item) => item.key === fileName);
      console.log(`determineNextEntryId(): indexClicked=${indexClicked}`);

      if (indexClicked < treeData.length - 1) {
        // no last entry
        const nextEntryId = treeData[indexClicked + 1].key;
        console.log(`determineNextEntryId(): nextEntryId=${nextEntryId}`);
        return dirName + '/' + nextEntryId;
      }
    } else {
      for (let entry of treeData) {
        if (entry.key === dirName && entry.nodes && entry.nodes.length > 0) {
          console.log(`determineNextEntryId(): entry=`, entry);
          indexClicked = entry.nodes.findIndex((item) => item.key === fileName);
          console.log(`determineNextEntryId(): indexClicked=${indexClicked}`);

          if (indexClicked < entry.nodes.length - 1) {
            // no last entry
            const nextEntryId = entry.nodes[indexClicked + 1].key;
            console.log(`determineNextEntryId(): nextEntryId=${nextEntryId}`);
            return dirName + '/' + nextEntryId;
          }
          break;
        }
      }
    }

    return null;
  };

  const fileEntryClicked = (key) => {
    console.log(`fileEntryClicked(): key=${key}`);

    if (key.indexOf("/") <= 0) {
      key = ROOT_DIRECTORY + "/" + key;
      console.log(`fileEntryClicked(): key=${key}`);
    }
    setEntryId(key);
    const dirName = key.split('/')[0];
    const fileName = key.split('/')[1];
    setDirName(dirName);
    setEntryName(fileName);
    setNewFileName(fileName);
    console.log(`fileEntryClicked(): dirName=${dirName}, fileName=${fileName}`);

    // reset
    setSuccessMessage('');
    setErrorMessage('');

    // determine nextEntryId
    const nextEntryId = determineNextEntryId(treeData, dirName, fileName);
    setNextEntryId(nextEntryId);

    // decompose file name to (author, title, extension)
    decomposeFileName(fileName);

    setViewUrl('/view/' + encodeURIComponent(dirName) + '/' + encodeURIComponent(fileName));
    setDownloadUrl(getUrlPrefix() + '/download/dirs/' + encodeURIComponent(dirName) + '/files/' + encodeURIComponent(fileName));

    // get file metadata (size, encoding)
    const fileMetadataUrl = '/dirs/' + encodeURIComponent(dirName) + '/files/' + encodeURIComponent(fileName);
    jsonGetReq(fileMetadataUrl, (result) => {
      setSize(result['size']);
      setEncoding(result['encoding']);
    }, (error) => {
      setErrorMessage(`file metadata load failed, ${error}`);
    });
  };

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

  const checkEntryExistence = (treeData, newDirName, newFileName) => {
    console.log('checkEntryExistence()');
    for (let entry of treeData) {
      // remove previous entry
      if (newDirName === ROOT_DIRECTORY) {
        if (entry.key === newFileName) {
          return true;
        }
      } else {
        if (entry.key === newDirName) {
          for (let node of entry.nodes) {
            if (node.key === newFileName) {
              return true;
            }
          }
        }
      }
    }
    return false;
  };

  const removeEntryFromTreeData = (treeData, dirName, fileName) => {
    let newTreeData = [...treeData];
    let isRemoved = false;
    // remove previous entry
    for (let entry of newTreeData) {
      if (dirName === ROOT_DIRECTORY) {
        if (entry.key === fileName) {
          newTreeData = newTreeData.filter(item => item !== entry);
          console.log(`removeEntryFromTreeData(): removed ${fileName}`);
          isRemoved = true;
        }
      } else {
        if (entry.key === dirName) {
          for (let node of entry.nodes) {
            if (node.key === fileName) {
              entry.nodes = entry.nodes.filter(item => item !== node);
              console.log(`removeEntryFromTreeData(): removed ${fileName}`);
              isRemoved = true;
            }

            if (isRemoved) {
              break;
            }
          }
        }
      }
    }
    return newTreeData;
  };

  const appendEntryToTreeData = (treeData, newDirName, newFileName) => {
    let newTreeData = [...treeData];
    let isAppended = false;
    // add new entry
    for (let entry of newTreeData) {
      if (newDirName === ROOT_DIRECTORY) {
        newTreeData.push({'key': newFileName, 'label': newFileName});
        console.log(`appendEntryToTreeData(): pushed ${newFileName} to root directory`);
        isAppended = true;
      } else {
        if (entry.key === newDirName) {
          entry.nodes.push({'key': newFileName, 'label': newFileName});
          console.log(`appendEntryToTreeData(): pushed ${newFileName} to ${newDirName}`);
          isAppended = true;
        }
      }

      if (isAppended) {
        break;
      }
    }
    return newTreeData;
  };

  const moveOrRenameFile = (dirName, fileName, newDirName, newFileName) => {
    console.log(`moveOrRenameFile: dirName=${dirName}, fileName=${fileName}, newDirName=${newDirName}, newFileName=${newFileName}`);
    if (checkEntryExistence(treeData, newDirName, newFileName) === false) {
      const moveOrRenameUrl = '/dirs/' + encodeURIComponent(dirName) + '/files/' + encodeURIComponent(fileName) + '/newdir/' + encodeURIComponent(newDirName) + '/newfile/' + encodeURIComponent(newFileName);
      console.log(moveOrRenameUrl);
      jsonPutReq(moveOrRenameUrl, () => {
        setSuccessMessage("파일 이름이나 위치가 변경되었습니다.");
        setErrorMessage('');

        let newTreeData = removeEntryFromTreeData(treeData, dirName, fileName);
        newTreeData = appendEntryToTreeData(newTreeData, newDirName, newFileName);
        setTreeData(newTreeData);
      }, (error) => {
        setErrorMessage(`파일 이름 변경에 실패했습니다. ${error}`);
      });
    } else {
      setErrorMessage("대상 디렉토리에 파일이 이미 존재합니다.");
    }
  };

  const changeButtonClicked = useCallback(() => {
    console.log(`changeButtonClicked: entryId=${entryId}, newFileName=${newFileName}`);
    const dirName = entryId.split('/')[0];
    const fileName = entryId.split('/')[1];
    moveOrRenameFile(dirName, fileName, dirName, newFileName);
  }, [entryId, newFileName]);

  const moveToUpperButtonClicked = useCallback(() => {
    console.log(`move to upper directory as '${newFileName}'`);
    const dirName = entryId.split('/')[0];
    const fileName = entryId.split('/')[1];
    moveOrRenameFile(dirName, fileName, ROOT_DIRECTORY, newFileName);
  }, [newFileName]);

  const selectDirectoryButtonClicked
    = useCallback((e, props) => {
    setSelectedDirectory(props);
  }, []);

  const moveToDirectoryButtonClicked
    = useCallback(() => {
    console.log(`move to '${selectedDirectory}' as '${newFileName}'`);
    const dirName = entryId.split('/')[0];
    const fileName = entryId.split('/')[1];
    moveOrRenameFile(dirName, fileName, selectedDirectory, newFileName);
  }, [newFileName, selectedDirectory]);

  const deleteButtonClicked = useCallback(() => {
    console.log(`deleteButtonClicked: entryId=${entryId}`);
    const dirName = entryId.split('/')[0];
    const fileName = entryId.split('/')[1];
    const deleteUrl = '/dirs/' + encodeURIComponent(dirName) + '/files/' + encodeURIComponent(fileName);
    console.log(deleteUrl);
    jsonDeleteReq(deleteUrl, () => {
      setSuccessMessage("파일이 삭제되었습니다.");
      setErrorMessage('');

      const newTreeData = removeEntryFromTreeData(treeData, dirName, fileName);
      setTreeData(newTreeData);

      if (nextEntryId) {
        console.log(`deleteButtonClicked(): nextEntryId=${nextEntryId}`);
        fileEntryClicked(nextEntryId);
      } else {
        setErrorMessage('마지막 파일입니다.');
      }
    }, (error) => {
      setErrorMessage(`파일 삭제에 실패했습니다. ${error}`);
    });
  }, [entryId, nextEntryId]);

  const toNextBookClicked = useCallback(() => {
    console.log(`toNextBookClicked: nextEntryId=${nextEntryId}`);
    if (nextEntryId) {
      console.log(`toNextBookClicked(): nextEntryId=${nextEntryId}`);
      fileEntryClicked(nextEntryId);
    } else {
      setErrorMessage('마지막 파일입니다.');
    }
  }, [nextEntryId]);

  return (
    <Container id="edit">
      <Row fluid="true">
        <Col md="3" lg="2" className="ps-0 pe-0 section">
          <Suspense fallback={<div className="loading">로딩 중...</div>}>
            <div style={{fontSize: '6pt'}}>Respond: {treeData && treeData['last_responded_time']}</div>
            <div style={{fontSize: '6pt'}}>Modify: {treeData && treeData['last_modified_time']}</div>
            <DirList treeData={treeData} onClickHandler={fileEntryClicked}/>
          </Suspense>
        </Col>

        <Col md="9" lg="10" className="section">
          <Row id="top_panel">
            <Col id="left_panel" lg="7" className="ps-0 pe-0">
              <Card>
                <Card.Header>
                  파일 정보
                </Card.Header>
                <Suspense fallback={<div className="loading">로딩 중...</div>}>
                  <Card.Body>
                    <Row>
                      <Col xs="3">
                        <Form.Control value={dirName} readOnly disabled/>
                      </Col>
                      <Col xs="9">
                        <Form.Control value={entryName} readOnly disabled/>
                      </Col>
                    </Row>
                    <Row>
                      <Col xs="3">
                        <InputGroup>
                          <InputGroup.Text>확장자</InputGroup.Text>
                          <Form.Control value={extension} onChange={extensionChanged}/>
                        </InputGroup>
                      </Col>
                      <Col xs="4">
                        <InputGroup>
                          <InputGroup.Text>인코딩</InputGroup.Text>
                          <Form.Control value={encoding} readOnly disabled/>
                        </InputGroup>
                      </Col>
                      <Col xs="5">
                        <InputGroup>
                          <InputGroup.Text>크기</InputGroup.Text>
                          <Form.Control value={size} readOnly disabled/>
                        </InputGroup>
                      </Col>
                    </Row>
                    <Row>
                      <InputGroup>
                        <InputGroup.Text>저자</InputGroup.Text>
                        <Form.Control value={author} onChange={authorChanged}/>
                        <Button variant="outline-secondary" size="sm" onClick={cutAuthorButtonClicked} disabled={!entryId}>
                          분할
                          <FontAwesomeIcon icon={faCut}/>
                        </Button>
                        <Button variant="outline-secondary" size="sm" onClick={exchangeButtonClicked} disabled={!entryId}>
                          교환
                          <FontAwesomeIcon icon={faRotate}/>
                        </Button>
                      </InputGroup>
                    </Row>
                    <Row>
                      <InputGroup>
                        <InputGroup.Text>제목</InputGroup.Text>
                        <Form.Control value={title} onChange={titleChanged}/>
                        <Button variant="outline-secondary" size="sm" onClick={cutTitleButtonClicked} disabled={!entryId}>
                          분할
                          <FontAwesomeIcon icon={faCut}/>
                        </Button>
                        <Button variant="outline-secondary" size="sm" onClick={resetButtonClicked} disabled={!entryId}>
                          복원
                          <FontAwesomeIcon icon={faClockRotateLeft}/>
                        </Button>
                      </InputGroup>
                    </Row>
                    <Row>
                      <Col>
                        <InputGroup>
                          <InputGroup.Text>신규 파일명</InputGroup.Text>
                          <Form.Control value={newFileName} onChange={newFileNameChanged}/>
                          <Button variant="outline-success" size="sm" onClick={changeButtonClicked} disabled={!entryId}>
                            변경
                            <FontAwesomeIcon icon={faCheck}/>
                          </Button>
                          <Button variant="outline-danger" size="sm" onClick={deleteButtonClicked} disabled={!entryId}>
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
                        <a href={`https://www.google.com/search?sourceid=chrome&ie=UTF-8&oq=${encodeURIComponent(author)}+${encodeURIComponent(title)}&q=${encodeURIComponent(author)}+${encodeURIComponent(title)}&sourceid=chrome&ie=UTF-8`} target="_blank" rel="noreferrer">
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
                      {
                        !entryId.startsWith(ROOT_DIRECTORY) > 0 &&
                        <Button variant="outline-warning" size="sm" onClick={moveToUpperButtonClicked} disabled={!newFileName}>
                          상위로
                          <FontAwesomeIcon icon={faUpload}/>
                        </Button>
                      }
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
                                      backgroundColor: getRandomMediumColor(category),
                                      color: 'white'
                                    }}
                                    onClick={(e) => {
                                      selectDirectoryButtonClicked
                                      (e, dir.key);
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
                                      selectDirectoryButtonClicked
                                      (e, dir.key);
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
                        <Button variant="outline-warning" size="sm" onClick={moveToDirectoryButtonClicked
                        } disabled={!entryId && !selectedDirectory}>
                          로 옮기기
                          <FontAwesomeIcon icon={faTruckMoving}/>
                        </Button>
                      </InputGroup>
                    </Row>
                  </Card.Body>
                </Suspense>
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

            <Col id="right_panel" lg="5" className="ps-0 pe-0">
              <Card>
                <Card.Header>
                  유사한 파일 목록
                </Card.Header>
                <Suspense fallback={<div className="loading">로딩 중...</div>}>
                  <Card.Body>
                  </Card.Body>
                </Suspense>
              </Card>

              <Card>
                <Card.Header>
                  검색 결과
                </Card.Header>
                <Suspense fallback={<div className="loading">로딩 중...</div>}>
                  <Card.Body>
                  </Card.Body>
                </Suspense>
              </Card>
            </Col>
          </Row>

          <Row id="bottom_panel">
            <Col id="right_panel" className="ps-0 pe-0">
              <Card>
                <Card.Header>
                  파일 보기
                  <span>
                    <a href={viewUrl} target="_blank" rel="noreferrer">
                      <Button variant="outline-primary" size="sm" disabled={!entryId} className="float-end">새 창에서 전체 보기</Button>
                    </a>
                    <a href={downloadUrl} target="_blank" rel="noreferrer">
                      <Button variant="outline-primary" size="sm" disabled={!entryId} className="float-end">다운로드</Button>
                    </a>
                  </span>
                </Card.Header>
                <Card.Body>
                  <ViewSingle key={entryId} entryId={entryId} lineCount={100} pageCount={10}/>
                </Card.Body>
              </Card>
            </Col>
          </Row>

        </Col>
      </Row>
    </Container>
  );
}
