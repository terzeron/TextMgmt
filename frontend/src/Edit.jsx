import './Edit.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {useCallback, useEffect, useState, Suspense} from 'react';
import {useParams, useOutletContext} from 'react-router-dom';

import {Alert, Button, Card, Col, Container, Form, InputGroup, Row} from 'react-bootstrap';
import {FontAwesomeIcon} from '@fortawesome/react-fontawesome';
import {faCheck, faTrash} from '@fortawesome/free-solid-svg-icons';

import {getApiUrlPrefix, jsonDeleteReq, jsonGetReq, jsonPutReq, ROOT_DIRECTORY} from './Common';
import Folder from './Folder';
import BookInfoView from './BookInfoView';
import Bookstore from './Bookstore';
import Actions from './Actions';
import SimilarBooks from './SimilarBooks';
import SearchResult from './SearchResult';
import ViewSingle from "./ViewSingle";
import {DateTime} from "luxon";


export default function Edit() {
    const { category: routeCategory, bookId: routeBookId } = useParams();
    const {searchResults, hasSearched} = useOutletContext();
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

    // 메시지 자동 사라짐 (10초 후)
    useEffect(() => {
        if (successMessage) {
            const timer = setTimeout(() => setSuccessMessage(''), 10000);
            return () => clearTimeout(timer);
        }
    }, [successMessage]);

    useEffect(() => {
        if (errorMessage) {
            const timer = setTimeout(() => setErrorMessage(''), 10000);
            return () => clearTimeout(timer);
        }
    }, [errorMessage]);

    useEffect(() => {
        const categoryListUrl = '/categories';
        jsonGetReq(categoryListUrl, null, (categoryList) => {
            // _root 카테고리(최상위 파일) 분리
            const hasRootFiles = categoryList.includes('_root');
            const nonEmptyCategories = categoryList.filter(c => c !== '_root');

            // 공통 prefix 찾기
            const findCommonPrefix = (strings) => {
                if (!strings || strings.length === 0) return '';
                if (strings.length === 1) {
                    const parts = strings[0].split('/');
                    return parts.length > 1 ? parts.slice(0, -1).join('/') + '/' : '';
                }
                const parts = strings.map(s => s.split('/'));
                const minLen = Math.min(...parts.map(p => p.length));
                let commonParts = [];
                for (let i = 0; i < minLen - 1; i++) {
                    const part = parts[0][i];
                    if (parts.every(p => p[i] === part)) {
                        commonParts.push(part);
                    } else {
                        break;
                    }
                }
                return commonParts.length > 0 ? commonParts.join('/') + '/' : '';
            };
            const commonPrefix = findCommonPrefix(nonEmptyCategories);

            // 폴더 목록 생성
            let data = nonEmptyCategories.sort((a, b) => a.localeCompare(b))
                .map(category => {
                    return {
                        id: category,
                        label: commonPrefix ? category.replace(commonPrefix, '') : category,
                        fileType: 'folder'
                    };
                });

            // 최상위 파일이 있으면 가져와서 추가
            if (hasRootFiles) {
                jsonGetReq('/categories/_root', null, (bookList) => {
                    const rootFiles = bookList
                        .sort((a, b) => a['title'].localeCompare(b['title']))
                        .map(book => ({
                            id: '/' + book['book_id'].toString(),
                            label: book['title'] + '.' + book['file_type'],
                            fileType: book['file_type'],
                            children: [],
                            book: book,
                        }));
                    setFolderData([...data, ...rootFiles]);
                }, () => {
                    setFolderData(data);
                });
            } else {
                setFolderData(data);
            }
            setCategoryList(categoryList);
        }, (error) => {
            setErrorMessage(`디렉토리 데이터를 불러올 수 없습니다. ${error}`);
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

    const decomposeTitle = useCallback((book) => {
        let title = '';
        let author = '';
        let extension = '';

        if (book['author'] !== '') {
            title = book['title'];
            author = book['author'];
            extension = book['file_type'];
            // title에 이미 저자 prefix가 포함된 경우 제거 (ES 데이터 오염 대응)
            const authorPrefix = '[' + author + '] ';
            if (title.startsWith(authorPrefix)) {
                title = title.substring(authorPrefix.length);
            }
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
        if (selectedFolderData && selectedFolderData.fileType === 'folder') {
            // category entry (폴더)
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
        } else if (selectedFolderData && selectedFolderData.book) {
            // 최상위 파일 (folderData에 직접 포함된 파일)
            const book = selectedFolderData.book;
            const bookId = book['book_id'];
            setSelectedEntryId(selectedEntryId);
            setOriginalBookInfo(book);
            const newBook = decomposeTitle(book);
            setBookInfo(newBook);
            setViewUrl('/view/' + book['file_type'] + '/' + bookId + '?path=' + encodeURIComponent(book['file_path']));
            setDownloadUrl(getApiUrlPrefix() + '/download/' + bookId);
            const otherCategoryList = categoryList
                .sort((a, b) => a.localeCompare(b))
                .filter(cat => cat !== '_root');
            setOtherCategoryList(otherCategoryList);
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
                    setViewUrl('/view/' + book['file_type'] + '/' + bookId + '?path=' + encodeURIComponent(book['file_path']));
                    setDownloadUrl(getApiUrlPrefix() + '/download/' + bookId);

                    // determine other category list
                    const otherCategoryList = categoryList
                        .sort((a, b) => a.localeCompare(b))
                        .filter(cat => cat !== category)
                    setOtherCategoryList(otherCategoryList);
                } else {
                    setErrorMessage(`선택한 책을 찾을 수 없습니다. (ID: ${bookId})`);
                }
            } else {
                setErrorMessage(`선택한 카테고리를 찾을 수 없습니다. (${category})`);
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

    useEffect(() => {
        if (routeCategory && routeBookId && folderData.length > 0) {
            const categoryItem = folderData.find(item => item.id === routeCategory);
            if (!categoryItem) return;
            // load children if not yet
            if (!categoryItem.children || categoryItem.children.length === 0) {
                entryClicked(routeCategory);
            } else {
                entryClicked(`${routeCategory}/${routeBookId}`);
            }
        }
    }, [routeCategory, routeBookId, folderData, entryClicked]);

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
        // newFileName에서 확장자 제거 후 비교용 label 생성
        const extensionSuffix = '.' + bookInfo['file_type'];
        const titleOnly = newFileName.endsWith(extensionSuffix)
            ? newFileName.slice(0, -extensionSuffix.length)
            : newFileName;
        const targetLabel = titleOnly + extensionSuffix;
        if (newDirName === '_root' || newDirName === '') {
            // 최상위 파일 확인
            return folderData.some(entry => entry.label === targetLabel);
        } else {
            // 폴더 내 파일 확인
            const folder = folderData.find(entry => entry.id === newDirName);
            if (folder && folder.children) {
                return folder.children.some(item => item.label === targetLabel);
            }
        }
        return false;
    }, [bookInfo]);

    const removeEntryFromFolderData = useCallback((folderData, dirName, fileName) => {
        let newFolderData = [...folderData];
        // 최상위 파일 제거 (_root 또는 /로 시작하는 id)
        if (dirName === '_root' || dirName === '') {
            const entryId = '/' + fileName;
            newFolderData = newFolderData.filter(item => item.id !== entryId);
            console.log(`removeEntryFromFolderData(): removed root file ${entryId}`);
        } else {
            // 폴더 내 파일 제거
            const entryId = dirName + '/' + fileName;
            newFolderData = newFolderData.map(entry => {
                if (entry.id === dirName && entry.children) {
                    return {
                        ...entry,
                        children: entry.children.filter(item => item.id !== entryId)
                    };
                }
                return entry;
            });
            console.log(`removeEntryFromFolderData(): removed ${entryId}`);
        }
        return newFolderData;
    }, []);

    const appendEntryToFolderData = useCallback((folderData, newDirName, newFileName) => {
        let newFolderData = [...folderData];
        const isRootFile = newDirName === '_root' || newDirName === '';
        // newFileName에서 확장자 제거 후 label 생성 (label은 파일명 전체)
        const extensionSuffix = '.' + bookInfo['file_type'];
        const labelWithoutExt = newFileName.endsWith(extensionSuffix)
            ? newFileName.slice(0, -extensionSuffix.length)
            : newFileName;
        const newEntry = {
            id: isRootFile ? '/' + bookInfo['book_id'] : newDirName + '/' + bookInfo['book_id'],
            label: labelWithoutExt + extensionSuffix,
            fileType: bookInfo['file_type'],
            children: [],
            // title과 author는 bookInfo 원본 유지 (titleOnly 사용 시 저자 중복 발생)
            book: { ...bookInfo, category: isRootFile ? '_root' : newDirName },
        };

        if (isRootFile) {
            // 최상위 파일 추가 (폴더들 뒤에 추가)
            const folderEndIndex = newFolderData.findIndex(item => item.fileType !== 'folder');
            if (folderEndIndex === -1) {
                newFolderData.push(newEntry);
            } else {
                newFolderData.splice(folderEndIndex, 0, newEntry);
            }
            console.log(`appendEntryToFolderData(): pushed ${newFileName} to root`);
        } else {
            // 폴더 내 파일 추가
            newFolderData = newFolderData.map(entry => {
                if (entry.id === newDirName && entry.children) {
                    return {
                        ...entry,
                        children: [...entry.children, newEntry]
                    };
                }
                return entry;
            });
            console.log(`appendEntryToFolderData(): pushed ${newFileName} to ${newDirName}`);
        }
        return newFolderData;
    }, [bookInfo]);

    const updateFile = useCallback((dirName, fileName, newDirName, newFileName) => {
        if (checkEntryExistence(folderData, newDirName, newFileName) === false) {
            const updateUrl = '/books/' + bookInfo['book_id'];
            // 백엔드는 '_root'를 기대하므로 빈 문자열을 '_root'로 변환
            const categoryForBackend = (newDirName === '' || newDirName === '_root') ? '_root' : newDirName;
            // newFileName에서 확장자 제거 (백엔드는 title에 확장자가 없어야 함)
            const extensionSuffix = '.' + bookInfo['file_type'];
            const titleOnly = newFileName.endsWith(extensionSuffix)
                ? newFileName.slice(0, -extensionSuffix.length)
                : newFileName;
            const newFilePath = categoryForBackend === '_root'
                ? titleOnly + extensionSuffix
                : newDirName + '/' + titleOnly + extensionSuffix;
            const updatedTime = DateTime.now().toFormat('yyyy-MM-dd\'T\'HH:mm:ss.SSS');
            const payload = { ...bookInfo, category: categoryForBackend, title: titleOnly, file_path: newFilePath, updated_time: updatedTime };
            jsonPutReq(updateUrl, payload, () => {
                // 구체적인 성공 메시지 생성
                const displayDirName = (dirName === '' || dirName === '_root') ? '최상위' : dirName;
                const displayNewDirName = (newDirName === '' || newDirName === '_root') ? '최상위' : newDirName;
                let message;
                if (dirName !== newDirName) {
                    message = `"${newFileName}"을(를) "${displayNewDirName}" 디렉토리로 이동했습니다.`;
                } else {
                    message = `"${displayDirName}"의 파일 이름을 "${newFileName}"(으)로 변경했습니다.`;
                }
                setSuccessMessage(message);
                setErrorMessage('');

                let newFolderData = removeEntryFromFolderData(folderData, dirName, fileName);
                newFolderData = appendEntryToFolderData(newFolderData, newDirName, newFileName);
                setFolderData(newFolderData);
            }, (error) => {
                setErrorMessage(`책 이름 변경에 실패했습니다. ${error}`);
            });
        } else {
            const displayNewDirName = (newDirName === '' || newDirName === '_root') ? '최상위' : newDirName;
            setErrorMessage(`"${displayNewDirName}" 디렉토리에 "${newFileName}"이(가) 이미 존재합니다.`);
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
            updateFile(dirName, fileName, '_root', newFileName);
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
            const bookId = selectedEntryId.split('/')[1];
            const deleteUrl = '/books/' + bookId;
            console.log(deleteUrl);
            jsonDeleteReq(deleteUrl, null, (response) => {
                const displayDirName = (dirName === '' || dirName === '_root') ? '최상위' : dirName;
                // warning이 있으면 경고 메시지와 함께 표시
                if (response?.warning) {
                    setSuccessMessage(`"${displayDirName}/${newFileName}"이(가) 삭제되었습니다. (경고: ${response.warning})`);
                } else {
                    setSuccessMessage(`"${displayDirName}/${newFileName}"이(가) 삭제되었습니다.`);
                }
                setErrorMessage('');

                const newFolderData = removeEntryFromFolderData(folderData, dirName, bookId);
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
    }, [selectedEntryId, nextEntryId, folderData, entryClicked, removeEntryFromFolderData, newFileName]);

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
                <Col md="3" lg="2" className="ps-0 pe-0 section directory-menu">
                    <Suspense fallback={<div className="loading">로딩 중...</div>}>
                        <Folder folderData={folderData} selectedItems={selectedItems} onClickHandler={entryClicked}/>
                    </Suspense>
                </Col>

                <Col md="9" lg="10" className="section">
                    {hasSearched &&
                        <SearchResult results={searchResults} showEditButton={true}/>
                    }
                    {bookInfo['book_id'] &&
                        <>
                            <Row id="top_panel">
                                <Col id="left_panel" md="6" lg="5" className="ps-0 pe-0">
                                    <Card>
                                        <Card.Header>
                                            책 정보
                                        </Card.Header>
                                        <Suspense fallback={<div className="loading">로딩 중...</div>}>
                                            <Card.Body>
                                                <BookInfoView bookInfo={bookInfo} isEditEnabled={true} onTitleChange={titleChanged} onAuthorChange={authorChanged} onCutTitleButtonClick={cutTitleButtonClicked} onCutAuthorButtonClick={cutAuthorButtonClicked} onExchangeButtonClick={exchangeButtonClicked} onResetButtonClick={resetButtonClicked} newFileName={newFileName} newFileNameChanged={newFileNameChanged} changeButtonClicked={changeButtonClicked} deleteButtonClicked={deleteButtonClicked}/>
                                            </Card.Body>
                                        </Suspense>
                                    </Card>

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
                                                            <Actions selectedEntryId={selectedEntryId} selectedCategory={selectedCategory} otherCategoryList={otherCategoryList} newFileName={newFileName} moveToUpperButtonClicked={moveToUpperButtonClicked} moveToDirectoryButtonClicked={moveToDirectoryButtonClicked} selectDirectoryButtonClicked={selectDirectoryButtonClicked} toNextEntryClicked={toNextEntryButtonClicked}/>
                                                        </Col>
                                                    </Row>

                                                    <Row className="button_group">
                                                        <Col>
                                                            <Bookstore bookInfo={bookInfo}/>
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
                                    <SimilarBooks bookId={bookInfo['book_id']} onSelect={entryClicked}/>
                                </Col>
                            </Row>

                            <Row id="bottom_panel">
                                <Col id="right_panel" className="ps-0 pe-0">
                                    <ViewSingle
                                        key={bookInfo['book_id']}
                                        bookId={bookInfo['book_id']}
                                        filePath={bookInfo['file_path']}
                                        fileType={bookInfo['file_type']}
                                        viewUrl={viewUrl}
                                        downloadUrl={downloadUrl}
                                        lineCount={100}
                                        pageCount={10}
                                    />
                                </Col>
                            </Row>
                        </>
                    }
                </Col>
            </Row>
        </Container>
    );
}
