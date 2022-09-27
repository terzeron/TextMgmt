import {useEffect, useRef, useState} from 'react';
import DirList from "./DirList";
import getUrlPrefix from "./Common";

import {Container, Row, Col, Card, Form, Button, InputGroup, Alert} from 'react-bootstrap';
import 'bootstrap/dist/css/bootstrap.min.css';
import './Edit.css';
import {FontAwesomeIcon} from "@fortawesome/react-fontawesome";
import {faCheck, faCheckDouble, faClockRotateLeft, faCodeCommit, faCut, faRotate, faSearch, faTrash, faTruckMoving, faUpload, faWheelchairMove} from "@fortawesome/free-solid-svg-icons";

export default function Edit() {
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState(null);

    const [dirInfo, _setDirInfo] = useState(null);
    const dirInfoRef = useRef(dirInfo);
    const setDirInfo = data => {
        dirInfoRef.current = data;
        _setDirInfo(data);
    };
    const [otherDirList, setOtherDirList] = useState([]);
    const [fileMetadata, setFileMetadata] = useState({
        "id": "",
        "name": ""
    });
    const [fileInfo, setFileInfo] = useState({
        "author": "",
        "title": "",
        "extension": "",
    });
    const [newFileName, setNewFileName] = useState(null);
    const [fileContent, setFileContent] = useState({
        "size": "",
        "encoding": "",
        "content": ""
    });
    const [fileContentLoading, setFileContentLoading] = useState(false);
    const [similarFileList, setSimilarFileList] = useState([]);
    const [similarFileListLoading, setSimilarFileListLoading] = useState(false);
    const [searchResult, setSearchResult] = useState(null);
    const [searchResultLoading, setSearchResultLoading] = useState(false);

    const fileClicked = (e, props) => {
        e.preventDefault();
        //console.log(`fileClicked(${props["id"]})`);
        const fileId = props["id"];
        const fileName = fileId.split("/").pop();
        const currentDirName = fileId.split("/").shift();
        //console.log(`fileId=${fileId}`);
        //console.log(`fileName=${fileName}`);
        //console.log(`currentDirName=${currentDirName}`);
        setFileMetadata({"id": fileId, "name": fileName});
        setNewFileName(fileName);
        decomposeFileName(fileName);

        const getFileContent = async (fileId, dirInfo) => {
            //console.log(`getFileContent(${fileId}, ${dirInfo})`);
            try {
                const dirName = fileId.split("/")[0]
                const fileName = fileId.split("/")[1]
                const response = await fetch(getUrlPrefix() + `/dirs/${dirName}/files/${fileName}`);
                if (!response.ok) {
                    throw new Error(`This is an HTTP error: The status is ${response.status}`);
                }
                const result = await response.json();
                if (result["status"] === "success") {
                    setFileContent(result["result"]);
                    setError(null);
                } else {
                    setError(result["error"]);
                }
                const otherDirList = dirInfo.filter((item) => item["items"] !== null || item["items"] !== [])
                    .filter((item) => item["id"] !== currentDirName)
                    .map((item) => {
                        const newItem = {
                            "id": item["id"]
                        }
                        return newItem;
                    });
                //console.log(otherDirList);
                setOtherDirList(otherDirList);
            } catch (err) {
                setError(err.message);
                setFileContent({
                    "size": "",
                    "encoding": "",
                    "content": ""
                });
            } finally {
                setLoading(false);
            }
        };
        //console.log(fileId);
        //console.log(dirInfoRef.current);
        getFileContent(fileId, dirInfoRef.current).then(() => {
            //console.log("file info loaded");
        }).catch((err) => {
            console.error("file info load failed");
            console.error(err);
        });
    };

    const cutAuthorClicked = (e, props) => {
        e.preventDefault();
        const tokens = fileInfo["author"].split(" ");
        setFileInfo({
            "author": tokens[0],
            "title": tokens[1],
            "extension": fileInfo["extension"]
        });
    };

    const cutTitleClicked = (e, props) => {
        e.preventDefault();
        const tokens = fileInfo["title"].split(" ");
        setFileInfo({
            "author": tokens[0],
            "title": tokens[1],
            "extension": fileInfo["extension"]
        });
    };

    const exchangeAuthorAndTitleClicked = (e, props) => {
        e.preventDefault();
        const tokens = [fileInfo["author"], fileInfo["title"]];
        setFileInfo({
            "author": tokens[1],
            "title": tokens[0],
            "extension": fileInfo["extension"]
        });
    };

    const resetAuthorAndTitleClicked = (e, props) => {
        e.preventDefault();
        decomposeFileName(fileMetadata["name"]);
    };

    const decomposeFileName = (fileName) => {
        //console.log(`decomposeFileName(${fileName})`);
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
        let author = "";
        let title = "";
        let extension = "";

        for (let pattern of [pattern1, pattern2, pattern3, pattern4, pattern5]) {
            const match = pattern.exec(fileName);
            if (match) {
                author = match.groups["author"] || "";
                title = match.groups["title"] || "";
                extension = match.groups["extension"] || "";
                setFileInfo({
                    "author": author,
                    "title": title,
                    "extension": extension,
                });
                //console.log(author, title, extension);
                break;
            }
        }
    }

    const showFileContent = () => {
        if (fileContentLoading)
            return <div>로딩 중...</div>
        else
            return <div>{fileContent.content}</div>;
    }

    const showSimilarFileList = () => {
        if (similarFileListLoading)
            return <div>로딩 중...</div>
        else
            return <div>{similarFileList}</div>
    }

    const showSearchResult = () => {
        if (searchResultLoading)
            return <div>로딩 중...</div>
        else
            return <div>{searchResult}</div>
    }

    const changeClicked = () => {
        alert("변경 버튼 클릭됨");
    }

    const moveToUpperDirectory = () => {
        alert("상위로 버튼 클릭됨");
    }

    const searchClicked = () => {
        alert("검색 버튼 클릭됨");
    }

    const removeClicked = () => {
        alert("삭제 버튼 클릭됨");
    }

    useEffect(() => {
        if (fileInfo && (fileInfo["author"] || fileInfo["title"])) {
            setNewFileName(`[${fileInfo.author}] ${fileInfo.title}.${fileInfo.extension}`);
        }
    }, [fileInfo]);

    useEffect(() => {
        const getDirListData = async () => {
            try {
                const response = await fetch(getUrlPrefix() + "/dirs/");
                if (!response.ok) {
                    throw new Error(`This is an HTTP error: The status is ${response.status}`);
                }
                const result = await response.json();
                const dirListData = result["result"].map((entry) => {
                    if ("items" in entry) {
                        // directory
                        entry["items"].map((subEntry) => {
                            if (!("items" in subEntry)) {
                                subEntry["onTitleClick"] = (e, props) => fileClicked(e, props);
                            }
                            return subEntry;
                        });
                    } else {
                        // file
                        entry["onTitleClick"] = (e, props) => fileClicked(e, props);
                    }
                    return entry;
                });
                setDirInfo(dirListData);
                setError(null);
            } catch (err) {
                setError(err.message);
                setDirInfo(null);
            } finally {
                setLoading(false);
            }
        }
        getDirListData().then(() => {
            //console.log("dir list data loaded");
        }).catch((err) => {
            console.error("dir list data load failed");
            console.error(err);
        });

        return () => {
            //console.log("컴퍼넌트가 사라질 때 cleanup할 일을 여기서 처리해야 함");
        };
    }, [] /* rendered once */);

    if (loading) return <div>로딩중..</div>;
    if (!dirInfo) return null;
    return (
        <Container id="edit">
            <Row fluid="true">
                <Col sm="3" lg="2" className="ps-0 pe-0">
                    <DirList data={dirInfo}/>
                </Col>

                <Col sm="9" lg="10">
                    <Row id="top_panel">
                        <Col id="left_panel" lg="5" className="ps-0 pe-0">
                            <Card>
                                <Card.Header>
                                    파일 정보
                                </Card.Header>
                                <Card.Body>
                                    <Row>
                                        <InputGroup>
                                            <InputGroup.Text>파일명</InputGroup.Text>
                                            <Form.Control defaultValue={fileMetadata.name} readOnly></Form.Control>
                                        </InputGroup>
                                    </Row>
                                    <Row>
                                        <Col xs="5">
                                            <InputGroup>
                                                <InputGroup.Text>인코딩</InputGroup.Text>
                                                <Form.Control defaultValue={fileContent.encoding} readOnly/>
                                            </InputGroup>
                                        </Col>
                                        <Col xs="7">
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
                                            <Button variant="outline-secondary" size="sm" onClick={cutAuthorClicked}>자르기 <FontAwesomeIcon icon={faCut}/></Button>
                                        </InputGroup>
                                    </Row>
                                    <Row>
                                        <InputGroup>
                                            <InputGroup.Text>제목</InputGroup.Text>
                                            <Form.Control defaultValue={fileInfo.title}/>
                                            <Button variant="outline-secondary" size="sm" onClick={cutTitleClicked}>자르기 <FontAwesomeIcon icon={faCut}/></Button>
                                        </InputGroup>
                                    </Row>
                                    <Row>
                                        <Col xs="4">
                                            <InputGroup>
                                                <InputGroup.Text>확장자</InputGroup.Text>
                                                <Form.Control defaultValue={fileInfo.extension}/>
                                            </InputGroup>
                                        </Col>
                                        <Col xs="8">
                                            <InputGroup>
                                                <InputGroup.Text>파일명</InputGroup.Text>
                                                <Form.Control defaultValue={newFileName}/>
                                            </InputGroup>
                                        </Col>
                                    </Row>
                                    <Row className="mt-2 button_group">
                                        <Col>
                                            <Button variant="warning" size="sm" onClick={changeClicked}>변경<FontAwesomeIcon icon={faCheck}/></Button>
                                            <Button variant="warning" size="sm" onClick={exchangeAuthorAndTitleClicked}>교환 <FontAwesomeIcon icon={faRotate}/></Button>
                                            <Button variant="warning" size="sm" onClick={moveToUpperDirectory}>상위로 <FontAwesomeIcon icon={faUpload}/></Button>
                                            <Button variant="warning" size="sm" onClick={resetAuthorAndTitleClicked}>초기화 <FontAwesomeIcon icon={faClockRotateLeft}/></Button>
                                            <Button variant="danger" size="sm" onClick={removeClicked}>삭제 <FontAwesomeIcon icon={faTrash}/></Button>
                                            <a href={`https://search.shopping.naver.com/book/search?bookTabType=ALL&pageIndex=1&pageSize=40&sort=REL&query=${fileInfo.author}+${fileInfo.title}`} target="_blank">
                                                <Button variant="primary" size="sm">네이버 검색</Button>
                                            </a>
                                            <a href={`https://www.google.com/search?sourceid=chrome&ie=UTF-8&oq=${fileInfo.author}+${fileInfo.title}&q=${fileInfo.author}+${fileInfo.title}`} target="_blank">
                                                <Button variant="primary" size="sm">구글 검색</Button>
                                            </a>
                                        </Col>
                                    </Row>
                                    <Row className="mt-2 button_group">
                                        {
                                            otherDirList.map((dir) =>
                                                <Button variant="outline-dark" size="sm" key={dir.id}>{`${dir.id}`}</Button>
                                            )
                                        }
                                    </Row>
                                    <Row className="mt-2">
                                        <InputGroup className="ms-0 me-0">
                                            <Form.Select></Form.Select>
                                            <Button variant="warning" size="sm">로 옮기기 <FontAwesomeIcon icon={faTruckMoving}/></Button>
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
                                        error && <Alert variant="danger" className="mb-0">{error}</Alert>
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
                                </Card.Header>
                                <Card.Body>
                                    {showFileContent()}
                                </Card.Body>
                            </Card>
                        </Col>
                    </Row>
                </Col>
            </Row>
        </Container>
    );
}