import {useEffect, useState, useCallback, Suspense} from 'react';
import {useParams, useOutletContext} from 'react-router-dom';

import {getApiUrlPrefix} from './Common';

import './View.css';
import 'bootstrap/dist/css/bootstrap.min.css';
import {Container, Row, Col, Card, Alert} from 'react-bootstrap';

import {jsonGetReq} from './Common';
import Folder from './Folder.jsx';
import ViewSingle from "./ViewSingle.jsx";
import BookInfoView from "./BookInfoView.jsx";
import SearchResult from './SearchResult';

// 모바일 감지 훅
function useIsMobile(breakpoint = 768) {
    const [isMobile, setIsMobile] = useState(
        typeof window !== 'undefined' ? window.innerWidth < breakpoint : false
    );

    useEffect(() => {
        const handleResize = () => setIsMobile(window.innerWidth < breakpoint);
        window.addEventListener('resize', handleResize);
        return () => window.removeEventListener('resize', handleResize);
    }, [breakpoint]);

    return isMobile;
}

export default function View() {
    const isMobile = useIsMobile();
    // get optional route params for deep link
    const { category: routeCategory, bookId: routeBookId } = useParams();
    const {searchResults, hasSearched, searchTotal, handleLoadMore, searchLoading} = useOutletContext();
    const [errorMessage, setErrorMessage] = useState('');
    const [successMessage, setSuccessMessage] = useState('');
    // removed selectedEntryId state as it's not needed
    const [folderData, setFolderData] = useState([]);
    const [bookInfo, setBookInfo] = useState({});
    const [viewUrl, setViewUrl] = useState('');
    const [downloadUrl, setDownloadUrl] = useState('');

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
        }, (error) => {
            setErrorMessage(`can't load directory data, ${error}`);
        });

        return () => {
            setErrorMessage('');
            setSuccessMessage('');
            // selectedEntryId state removed
            setFolderData([]);
            setBookInfo({});
            setViewUrl('');
            setDownloadUrl('');
        }
    }, []);

    const entryClicked = useCallback((selectedEntryId) => {
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
            setBookInfo(book);
            setViewUrl('/viewer/' + book['file_type'] + '/' + bookId + '?path=' + encodeURIComponent(book['file_path']));
            setDownloadUrl(getApiUrlPrefix() + '/download/' + bookId);
        } else {
            // book entry (폴더 내 파일)
            const category = selectedEntryId.split('/')[0];
            const bookId = selectedEntryId.split('/')[1];
            const booksInCategory = folderData.find(categoryItem => categoryItem.id === category)?.children;
            if (booksInCategory) {
                const book = booksInCategory.find(bookItem => bookItem.id === selectedEntryId)?.book;
                if (book) {
                    setBookInfo(book);
                    setViewUrl('/viewer/' + book['file_type'] + '/' + bookId + '?path=' + encodeURIComponent(book['file_path']));
                    setDownloadUrl(getApiUrlPrefix() + '/download/' + bookId);
                } else {
                    setErrorMessage(`can't find the selected book`);
                }
            } else {
                setErrorMessage(`can't find the selected category`);
            }
        }
    }, [folderData]);

    // if route specifies a category/bookId, auto-select after folderData loads
    useEffect(() => {
        if (routeCategory && routeBookId && folderData.length > 0) {
            // _root 카테고리는 folderData에서 /{bookId} 형식으로 저장됨
            if (routeCategory === '_root') {
                entryClicked('/' + routeBookId);
                return;
            }
            const categoryItem = folderData.find(item => item.id === routeCategory);
            if (!categoryItem) return;
            // If category children not loaded, load them first
            if (!categoryItem.children || categoryItem.children.length === 0) {
                entryClicked(routeCategory);
            } else {
                // Children already loaded, select specific book
                entryClicked(`${routeCategory}/${routeBookId}`);
            }
        }
    }, [routeCategory, routeBookId, folderData, entryClicked]);

    // 모바일에서는 directory-menu 클래스를 제거하여 고정 높이 스타일 방지
    const directoryClassName = isMobile
        ? "ps-0 pe-0"
        : "ps-0 pe-0 section directory-menu";

    return (
        <Container id="view">
            <Row fluid="true">
                <Col md={isMobile ? 12 : 3} lg={isMobile ? 12 : 2} className={directoryClassName}>
                    <Suspense fallback={<div className="loading">로딩 중...</div>}>
                        <Folder folderData={folderData} onClickHandler={entryClicked}/>
                    </Suspense>
                </Col>

                <Col md={isMobile ? 12 : 9} lg={isMobile ? 12 : 10} className={isMobile ? "ps-0 pe-0" : "section"}>
                    {hasSearched &&
                        <SearchResult
                            results={searchResults}
                            showEditButton={false}
                            onLoadMore={handleLoadMore}
                            hasMore={searchResults.length < searchTotal}
                            loading={searchLoading}
                        />
                    }
                    {bookInfo['book_id'] &&
                        <>
                            <Row id="top_panel">
                                <Col lg="12" className="ps-0 pe-0 me-0 ">
                                    <BookInfoView bookInfo={bookInfo} isEditEnabled={false}/>
                                </Col>

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
                            </Row>

                            <Row id="bottom_panel">
                                <Col id="right_panel" className="ps-0 pe-0">
                                    {
                                        bookInfo['book_id'] &&
                                        <ViewSingle key={bookInfo['book_id']} bookId={bookInfo['book_id']} filePath={bookInfo['file_path']} fileType={bookInfo['file_type']} viewUrl={viewUrl} downloadUrl={downloadUrl} lineCount={100} pageCount={10}/>
                                    }
                                </Col>
                            </Row>
                        </>
                    }
                </Col>
            </Row>
        </Container>
    );
}
