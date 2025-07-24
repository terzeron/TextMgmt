import './Edit.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {useCallback, useEffect, useState, Suspense} from 'react';

import {Alert, Button, Card, Col, Container, Form, InputGroup, Row} from 'react-bootstrap';
import {FontAwesomeIcon} from '@fortawesome/react-fontawesome';
import {faCheck, faTrash} from '@fortawesome/free-solid-svg-icons';

import {getApiUrlPrefix, jsonDeleteReq, jsonGetReq, jsonPutReq, ROOT_DIRECTORY} from './Common';
import Folder from './Folder';
import BookInfoView from './BookInfoView';
import Actions from './Actions';
import Move from './Move';
import SimilarBooks from './SimilarBooks';
import SearchResult from './SearchResult';
import ViewSingle from "./ViewSingle";
import {DateTime} from "luxon";


export default function Edit() {
    const [errorMessage, setErrorMessage] = useState('');
    const [successMessage, setSuccessMessage] = useState('');

    const [folderData, setFolderData] = useState([]);
    const [categoryList, setCategoryList] = useState([]);
    const [otherCategoryList, setOtherCategoryList] = useState([]);
    const [selectedEntryId, setSelectedEntryId] = useState('');
    const [selectedItems, setSelectedItems] = useState([]);
    const [nextEntryId, setNextEntryId] = useState('');
    const [selectedCategory, setSelectedCategory] = useState('');

    const [originalBookInfo, setOriginalBookInfo] = useState({});
    const [bookInfo, setBookInfo] = useState({});
    const [newFileName, setNewFileName] = useState('');
    const [viewUrl, setViewUrl] = useState('');
    const [downloadUrl, setDownloadUrl] = useState('');

    useEffect(() => {
        const categoryListUrl = '/categories';
        jsonGetReq(categoryListUrl, null, (categoryList) => {
            let data = categoryList.sort((a, b) => a.localeCompare(b))
                .map(category => {
                    return {
                        id: category,
                        label: category,
                        fileType: 'folder'
                    };
                });
            setFolderData(data);
            setCategoryList(categoryList);
        }, (error) => {
            setErrorMessage(`can't load directory data, ${error}`);
        });

        return () => {
            setFolderData([]);
            setOtherCategoryList([]);
            setSelectedEntryId('');

            setBookInfo({});
            setNewFileName('');
            setViewUrl('');
            setDownloadUrl('');
        };
    }, [] /* rendered once */);

    useEffect(() => {
        console.log(`bookInfo=${JSON.stringify(bookInfo)}`);
        // determine new file name from author and title
        let newName = '';
        if (bookInfo['author']) {
            newName = '[' + bookInfo['author'] + '] ';
        }
        if (bookInfo['title']) {
            newName += bookInfo['title'] + '.' + bookInfo['file_type'];
        }
        setNewFileName(newName);
    }, [bookInfo, setNewFileName]);

    const decomposeTitle = useCallback((book) => {
        let title = '';
        let author = '';
        let extension = '';

        if (book['author'] !== '') {
            title = book['title'];
            author = book['author'];
            extension = book['file_type'];
        } else {
            const name = book['title'] + '.' + book['file_type'];
            // [ 저자 ] 제목 . 확장자
            const pattern1 = /^\s*\[(?<author>.*?)]\s*(?<title>.*?)\s*\.(?<extension>\w+)\s*$/;
            // ( 저자 ) 제목 . 확장자
            const pattern2 = /^\s*\((?<author>.*?)\)\s*(?<title>.*?)\s*\.(?<extension>\w+)\s*$/;
            // 제목 [ 저자 ] . 확장자
            const pattern3 = /^\s*(?<title>.*?)\s*\[\s*(?<author>.*?)\s*]\s*\.(?<extension>\w+)\s*$/;
            // 제목 @ 저자 . 확장자
            const pattern4 = /^\s*(?<title>.*?)\s*@\s*(?<author>.*?)\s*\.(?<extension>\w+)\s*$/;
            // 저자 - 제목 . 확장자 or 저자 _ 제목 . 확장자
            const pattern5 = /^\s*(?<author>.*?)\s*[_-]\s*(?<title>.*?)\s*\.(?<extension>\w+)\s*$/;
            // 제목 ( 저자 )
            const pattern6 = /^\s*(?<title>.*?)\s*\(\s*(?<author>.*?)\s*\)\s*\.(?<extension>\w+)\s*$/;
            // 모두 제목으로 간주
            const finalPattern = /^(?<title>.+)\.(?<extension>\w+)\s*$/;

            for (const pattern of [pattern1, pattern2, pattern3, pattern4, pattern5, pattern6, finalPattern]) {
                const match = pattern.exec(name);
                if (match) {
                    title = match.groups.title || '';
                    author = match.groups.author || '';
                    extension = match.groups.extension || '';
                    break;
                }
            }
        }
        return {...book, title: title, author: author, file_type: extension};
    }, []);

    const entryClicked = useCallback((selectedEntryId) => {
        const determineNextEntryId = (folderData, selectedEntryId) => {
            /*
            let indexClicked = null;
            if (dirName === ROOT_DIRECTORY) {
                indexClicked = folderData.findIndex((item) => item.key === fileName);
                console.log(`determineNextEntryId(): indexClicked=${indexClicked}`);

                if (indexClicked < folderData.length - 1) {
                    // no last entry
                    const nextEntryId = folderData[indexClicked + 1].key;
                    console.log(`determineNextEntryId(): nextEntryId=${nextEntryId}`);
                    return dirName + '/' + nextEntryId;
                }
            } else {
                for (let entry of folderData) {
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
            */
            const category = selectedEntryId.split('/')[0];
            const bookId = selectedEntryId.split('/')[1];
            if (bookId) {
                const children = folderData.find(obj => obj.id === category)?.children;
                if (children) {
                    const index = children.findIndex(item => item.id === selectedEntryId);
                    if (0 <= index && index < (children.length - 2)) {
                        return children[index + 1].id;
                    }
                }
            }
            return null;
        };

        const selectedFolderData = folderData.find(o => o.id === selectedEntryId);
        if (selectedFolderData) {
            // category entry
            const booksInCategoryUrl = '/categories/' + selectedEntryId;
            const isChildrenLoaded = folderData.find(item => item.id === selectedEntryId && item.children && item.children.length > 0)
            if (!isChildrenLoaded) {
                jsonGetReq(booksInCategoryUrl, null, (bookList) => {
                    const data = folderData.map(item => {
                        if (item.id === selectedEntryId) {
                            // add book list to the selected category
                            return {
                                ...item,
                                children: bookList
                                    .sort((a, b) => a['title'].localeCompare(b['title']))
                                    .map(book => {
                                        return {
                                            id: item.id + '/' + book['book_id'].toString(),
                                            label: book['title'] + '.' + book['file_type'],
                                            fileType: book['file_type'],
                                            children: [],
                                            book: book,
                                        }
                                    })
                            }
                        } else {
                            return item;
                        }
                    });
                    setFolderData(data);
                });
            }
        } else {
            // book entry
            const category = selectedEntryId.split('/')[0];
            const bookId = selectedEntryId.split('/')[1];
            const booksInCategory = folderData.find(categoryItem => categoryItem.id === category)?.children;
            setSelectedEntryId(selectedEntryId);
            if (booksInCategory) {
                const book = booksInCategory.find(bookItem => bookItem.id === selectedEntryId)?.book;
                if (book) {
                    // save as original
                    setOriginalBookInfo(book);

                    // decompose file name to (author, title, extension)
                    const newBook = decomposeTitle(book);
                    setBookInfo(newBook);
                    setViewUrl('/view/' + book['file_type'] + '/' + bookId + '/' + encodeURIComponent(book['file_path']));
                    setDownloadUrl(getApiUrlPrefix() + '/download/' + bookId + '/' + encodeURIComponent(book['file_path']));

                    // determine other category list
                    const otherCategoryList = categoryList
                        .sort((a, b) => a.localeCompare(b))
                        .filter(cat => cat !== category)
                    setOtherCategoryList(otherCategoryList);
                } else {
                    setErrorMessage(`can't find the selected book`);
                }
            } else {
                setErrorMessage(`can't find the selected category`);
            }
        }

        /*
        if (selectedEntryId.indexOf('/') <= 0) {
            selectedEntryId = ROOT_DIRECTORY + '/' + selectedEntryId;
            console.log(`entryClicked(): key=${selectedEntryId}`);
        }
        */

        // determine nextEntryId
        const nextEntryId = determineNextEntryId(folderData, selectedEntryId);
        setNextEntryId(nextEntryId);
    }, [folderData, categoryList, decomposeTitle]);

    const newFileNameChanged = useCallback((e) => {
        setNewFileName(e.target.value);
    }, []);

    const titleChanged = useCallback((e) => {
        if (e?.target) {
            const title = e.target.value;
            setBookInfo({...bookInfo, title: title});
        }
    }, [bookInfo]);

    const authorChanged = useCallback((e) => {
        if (e?.target) {
            const author = e.target.value;
            setBookInfo({...bookInfo, author: author});
        }
    }, [bookInfo]);

    const cutTitleButtonClicked = useCallback(() => {
        if (bookInfo['title']?.includes(' ')) {
            const tokens = bookInfo['title'].split(' ');
            const newAuthor = tokens[0];
            const newTitle = tokens[1];
            setBookInfo({...bookInfo, author: newAuthor, title: newTitle});
        }
    }, [bookInfo]);

    const cutAuthorButtonClicked = useCallback(() => {
        if (bookInfo['author']?.includes(' ')) {
            const tokens = bookInfo['author'].split(' ');
            const newAuthor = tokens[0];
            const newTitle = tokens[1];
            setBookInfo({...bookInfo, author: newAuthor, title: newTitle});
        }
    }, [bookInfo]);

    const exchangeButtonClicked = useCallback(() => {
        const newAuthor = bookInfo['title'];
        const newTitle = bookInfo['author'];
        setBookInfo({...bookInfo, author: newAuthor, title: newTitle});
    }, [bookInfo]);

    const resetButtonClicked = useCallback(() => {
        const newBook = decomposeTitle(originalBookInfo);
        setBookInfo(newBook);
    }, [originalBookInfo, decomposeTitle]);

    const checkEntryExistence = useCallback((folderData, newDirName, newFileName) => {
        console.log(`checkEntryExistence(newDirName=${newDirName}, newFileName=${newFileName})`);
        for (let entry of folderData) {
            // remove previous entry
            if (newDirName === ROOT_DIRECTORY) {
                if (entry.key === newFileName) {
                    return true;
                }
            } else if (entry.key === newDirName) {
                for (let node of entry.nodes) {
                    if (node.key === newFileName) {
                        return true;
                    }
                }
            }
        }
        return false;
    }, []);

    const removeEntryFromFolderData = useCallback((folderData, dirName, fileName) => {
        let newFolderData = [...folderData];
        let isRemoved = false;
        // remove previous entry
        for (let entry of newFolderData) {
            if (dirName === ROOT_DIRECTORY) {
                if (entry.key === fileName) {
                    newFolderData = newFolderData.filter(item => item !== entry);
                    console.log(`removeEntryFromFolderData(): removed ${fileName}`);
                    isRemoved = true;
                }
            } else if (entry.key === dirName) {
                for (let node of entry.nodes) {
                    if (node.key === fileName) {
                        entry.nodes = entry.nodes.filter(item => item !== node);
                        console.log(`removeEntryFromFolderData(): removed ${fileName}`);
                        isRemoved = true;
                    }

                    if (isRemoved) {
                        break;
                    }
                }
            }
        }
        return newFolderData;
    }, []);

    const appendEntryToFolderData = useCallback((folderData, newDirName, newFileName) => {
        let newFolderData = [...folderData];
        let isAppended = false;
        // add new entry
        for (let entry of newFolderData) {
            if (newDirName === ROOT_DIRECTORY) {
                newFolderData.push({key: newFileName, label: newFileName});
                console.log(`appendEntryToFolderData(): pushed ${newFileName} to root directory`);
                isAppended = true;
            } else if (entry.key === newDirName) {
                entry.nodes.push({key: newFileName, label: newFileName});
                console.log(`appendEntryToFolderData(): pushed ${newFileName} to ${newDirName}`);
                isAppended = true;
            }

            if (isAppended) {
                break;
            }
        }
        return newFolderData;
    }, []);

    const updateFile = useCallback((dirName, fileName, newDirName, newFileName) => {
        if (checkEntryExistence(folderData, newDirName, newFileName) === false) {
            const updateUrl = '/books/' + bookInfo['book_id'];
            const newFilePath = newDirName + '/' + newFileName + '.' + bookInfo['file_type'];
            const updatedTime = DateTime.now().toFormat('yyyy-MM-dd\'T\'HH:mm:ss.SSS');
            const payload = { ...bookInfo, category: newDirName, title: newFileName, file_path: newFilePath, updated_time: updatedTime };
            jsonPutReq(updateUrl, payload, () => {
                setSuccessMessage("책 이름이나 위치가 변경되었습니다.");
                setErrorMessage('');

                let newFolderData = removeEntryFromFolderData(folderData, dirName, fileName);
                newFolderData = appendEntryToFolderData(newFolderData, newDirName, newFileName);
                setFolderData(newFolderData);
            }, (error) => {
                setErrorMessage(`책 이름 변경에 실패했습니다. ${error}`);
            });
        } else {
            setErrorMessage("대상 디렉토리에 책이 이미 존재합니다.");
        }
    }, [bookInfo, folderData, checkEntryExistence, appendEntryToFolderData, removeEntryFromFolderData]);

    const changeButtonClicked = useCallback(() => {
        console.log(`changeButtonClicked: selectedEntryId=${selectedEntryId}, newFileName=${newFileName}`);
        if (selectedEntryId?.includes('/')) {
            const dirName = selectedEntryId.split('/')[0];
            const fileName = selectedEntryId.split('/')[1];
            updateFile(dirName, fileName, dirName, newFileName);
        }
    }, [updateFile, selectedEntryId, newFileName]);

    const moveToUpperButtonClicked = useCallback(() => {
        console.log(`move to upper directory as '${newFileName}'`);
        if (selectedEntryId?.includes('/')) {
            const dirName = selectedEntryId.split('/')[0];
            const fileName = selectedEntryId.split('/')[1];
            updateFile(dirName, fileName, ROOT_DIRECTORY, newFileName);
        }
    }, [updateFile, selectedEntryId, newFileName]);

    const selectDirectoryButtonClicked = useCallback((e, props) => {
        setSelectedCategory(props);
    }, []);

    const moveToDirectoryButtonClicked = useCallback(() => {
        console.log(`move to '${selectedCategory}' as '${newFileName}'`);
        if (selectedEntryId?.includes('/')) {
            const dirName = selectedEntryId.split('/')[0];
            const fileName = selectedEntryId.split('/')[1];
            updateFile(dirName, fileName, selectedCategory, newFileName);
        }
    }, [updateFile, selectedEntryId, selectedCategory, newFileName]);

    const deleteButtonClicked = useCallback(() => {
        console.log(`deleteButtonClicked: entryId=${selectedEntryId}`);
        if (selectedEntryId?.includes('/')) {
            const dirName = selectedEntryId.split('/')[0];
            const fileName = selectedEntryId.split('/')[1];
            const deleteUrl = '/dirs/' + encodeURIComponent(dirName) + '/files/' + encodeURIComponent(fileName);
            console.log(deleteUrl);
            jsonDeleteReq(deleteUrl, null, () => {
                setSuccessMessage("책이 삭제되었습니다.");
                setErrorMessage('');

                const newFolderData = removeEntryFromFolderData(folderData, dirName, fileName);
                setFolderData(newFolderData);

                if (nextEntryId) {
                    console.log(`deleteButtonClicked(): nextEntryId=${nextEntryId}`);
                    entryClicked(nextEntryId);
                } else {
                    setErrorMessage('마지막 책입니다.');
                }
            }, (error) => {
                setErrorMessage(`책 삭제에 실패했습니다. ${error}`);
            });
        }
    }, [selectedEntryId, nextEntryId, folderData, entryClicked, removeEntryFromFolderData]);

    const toNextEntryButtonClicked = useCallback(() => {
        console.log(`toNextEntryButtonClicked: nextEntryId=${nextEntryId}`);
        if (nextEntryId) {
            setSelectedItems([nextEntryId]);
            entryClicked(nextEntryId);
        } else {
            setErrorMessage('마지막 책입니다.');
        }
    }, [nextEntryId, entryClicked]);

    return (
        <Container id="edit">
            <Row fluid="true">
                <Col md="3" lg="2" className="ps-0 pe-0 section">
                    <Suspense fallback={<div className="loading">로딩 중...</div>}>
                        <Folder folderData={folderData} selectedItems={selectedItems} onClickHandler={entryClicked}/>
                    </Suspense>
                </Col>

                <Col md="9" lg="10" className="section">
                    {
                        !bookInfo['book_id'] &&
                        <Card.Body>책이 선택되지 않았습니다.</Card.Body>
                    }
                    {
                        bookInfo['book_id'] &&
                        <>
                            <Row id="top_panel">
                                <Col id="left_panel" md="6" lg="5" className="ps-0 pe-0">
                                    <BookInfoView bookInfo={bookInfo} isEditEnabled={true} onTitleChange={titleChanged} onAuthorChange={authorChanged} onCutTitleButtonClick={cutTitleButtonClicked} onCutAuthorButtonClick={cutAuthorButtonClicked} onExchangeButtonClick={exchangeButtonClicked} onResetButtonClick={resetButtonClicked}/>

                                    <Card>
                                        <Suspense fallback={<div className="loading">로딩 중...</div>}>
                                            {
                                                <Card.Body>
                                                    <Row>
                                                        <Col>
                                                            <InputGroup>
                                                                <InputGroup.Text>신규 이름</InputGroup.Text>
                                                                <Form.Control value={newFileName} onChange={newFileNameChanged}/>
                                                                <Button variant="outline-success" size="sm" onClick={changeButtonClicked} disabled={!selectedEntryId}>
                                                                    변경
                                                                    <FontAwesomeIcon icon={faCheck}/>
                                                                </Button>
                                                                <Button variant="outline-danger" size="sm" onClick={deleteButtonClicked} disabled={!selectedEntryId}>
                                                                    삭제
                                                                    <FontAwesomeIcon icon={faTrash}/>
                                                                </Button>
                                                            </InputGroup>
                                                        </Col>
                                                    </Row>

                                                    <Row className="button_group">
                                                        <Col>
                                                            <Actions toNextEntryClicked={toNextEntryButtonClicked} bookInfo={bookInfo}/>
                                                        </Col>
                                                    </Row>

                                                    <Row className="button_group">
                                                        <Col>
                                                            <Move selectedEntryId={selectedEntryId} selectedCategory={selectedCategory} otherCategoryList={otherCategoryList} newFileName={newFileName} moveToUpperButtonClicked={moveToUpperButtonClicked} moveToDirectoryButtonClicked={moveToDirectoryButtonClicked} selectDirectoryButtonClicked={selectDirectoryButtonClicked}/>
                                                        </Col>
                                                    </Row>
                                                </Card.Body>
                                            }
                                        </Suspense>
                                    </Card>

                                    <Card>
                                        <Card.Header>
                                            실행 결과
                                        </Card.Header>
                                        <Card.Body>
                                            {
                                                errorMessage &&
                                                <Alert variant="danger" className="mb-0">{errorMessage}</Alert>
                                            }
                                            {
                                                successMessage &&
                                                <Alert variant="success" className="mb-0">{successMessage}</Alert>
                                            }
                                        </Card.Body>
                                    </Card>
                                </Col>

                                <Col id="right_panel" md="6" lg="7" className="ps-0 pe-0">
                                    <SimilarBooks/>
                                    <SearchResult/>
                                </Col>
                            </Row>

                            <Row id="bottom_panel">
                                <Col id="right_panel" className="ps-0 pe-0">
                                    <ViewSingle key={bookInfo['book_id']} bookId={bookInfo['book_id']} filePath={bookInfo['file_path']} fileType={bookInfo['file_type']} viewUrl={viewUrl} downloadUrl={downloadUrl} lineCount={100} pageCount={10}/>
                                </Col>
                            </Row>
                        </>
                    }
                </Col>
            </Row>
        </Container>
    );
}
