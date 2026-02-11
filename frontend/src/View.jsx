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
import {findCommonPrefix, buildFolderHierarchy, parseEntryId, findFolderInTree, updateFolderInTree} from './folderUtils';

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
    const params = useParams();
    const routeWildcard = params['*'] || '';
    const routeParsed = routeWildcard ? parseEntryId(routeWildcard) : null;
    const routeCategory = routeParsed?.category;
    const routeBookId = routeParsed?.bookId;
    const {searchResults, hasSearched, role, searchTotal, handleLoadMore, searchLoading} = useOutletContext();
    const [isFolderOpen, setIsFolderOpen] = useState(false);
    const [errorMessage, setErrorMessage] = useState('');
    const [successMessage, setSuccessMessage] = useState('');
    // removed selectedEntryId state as it's not needed
    const [folderData, setFolderData] = useState([]);
    const [bookInfo, setBookInfo] = useState({});
    const [viewUrl, setViewUrl] = useState('');
    const [downloadUrl, setDownloadUrl] = useState('');

    useEffect(() => {
        const categoryListUrl = '/categories';
        jsonGetReq(categoryListUrl, null, (categoryCounts) => {
            // categoryCounts: {"_epub": 5, "_pdf": 3, "_root": 2, ...}
            const categoryList = Object.keys(categoryCounts);

            // _root 카테고리(최상위 파일) 분리
            const hasRootFiles = categoryList.includes('_root');
            const nonEmptyCategories = categoryList.filter(c => c !== '_root');

            const buildAndSetFolderData = (filteredCategories, counts) => {
                const commonPrefix = findCommonPrefix(filteredCategories);

                // 2단계 계층 구조 생성
                let data = buildFolderHierarchy(
                    filteredCategories.sort((a, b) => a.localeCompare(b)),
                    commonPrefix,
                    counts
                );

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
            };

            // viewer인 경우 비노출 카테고리 필터링
            if (role === 'viewer') {
                jsonGetReq('/hidden-categories', null, (hiddenList) => {
                    const hiddenSet = new Set(hiddenList || []);
                    const filteredCategories = nonEmptyCategories.filter(cat => {
                        for (const hidden of hiddenSet) {
                            if (cat === hidden || cat.startsWith(hidden + '/')) {
                                return false;
                            }
                        }
                        return true;
                    });
                    buildAndSetFolderData(filteredCategories, categoryCounts);
                }, () => {
                    // hidden 목록 로드 실패 시 전체 카테고리 표시
                    buildAndSetFolderData(nonEmptyCategories, categoryCounts);
                });
            } else {
                buildAndSetFolderData(nonEmptyCategories, categoryCounts);
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
    }, [role]);

    const entryClicked = useCallback((selectedEntryId) => {
        // 2단계 트리에서 검색
        const selectedFolderData = findFolderInTree(folderData, selectedEntryId);
        if (selectedFolderData && selectedFolderData.fileType === 'folder') {
            // category entry (폴더)

            // 가상 부모 클릭 시 API 호출 안 함
            if (selectedFolderData.isVirtualParent) {
                return;
            }

            const booksInCategoryUrl = '/categories/' + selectedEntryId;
            // booksLoaded 플래그로 중복 로딩 방지
            if (!selectedFolderData.booksLoaded) {
                jsonGetReq(booksInCategoryUrl, null, (bookList) => {
                    const bookEntries = bookList
                        .sort((a, b) => a['title'].localeCompare(b['title']))
                        .map(book => ({
                            id: selectedEntryId + '/' + book['book_id'].toString(),
                            label: book['title'] + '.' + book['file_type'],
                            fileType: book['file_type'],
                            children: [],
                            book: book,
                        }));

                    const data = updateFolderInTree(folderData, selectedEntryId, (item) => {
                        // 기존 하위 폴더 children 보존 후 책 추가
                        const existingSubfolders = (item.children || []).filter(c => c.fileType === 'folder');
                        return {
                            ...item,
                            booksLoaded: true,
                            children: [...existingSubfolders, ...bookEntries],
                        };
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
            const parsed = parseEntryId(selectedEntryId);
            if (!parsed) return;
            const category = parsed.category;
            const bookId = parsed.bookId;
            const folder = findFolderInTree(folderData, category);
            const booksInCategory = folder?.children;
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
            const categoryItem = findFolderInTree(folderData, routeCategory);
            if (!categoryItem) {
                // 폴더 트리에 없는 경로 (3레벨 이상) - 백엔드에서 직접 조회
                jsonGetReq('/books/' + routeBookId, null, (book) => {
                    setBookInfo(book);
                    setViewUrl('/viewer/' + book['file_type'] + '/' + routeBookId + '?path=' + encodeURIComponent(book['file_path']));
                    setDownloadUrl(getApiUrlPrefix() + '/download/' + routeBookId);
                }, (error) => {
                    setErrorMessage(`책 정보를 불러올 수 없습니다. ${error}`);
                });
                return;
            }
            // If category children not loaded, load them first
            if (!categoryItem.booksLoaded) {
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
                {isFolderOpen && (
                    <Col md={isMobile ? 12 : 3} lg={isMobile ? 12 : 2} className={directoryClassName}>
                        <Suspense fallback={<div className="loading">로딩 중...</div>}>
                            <Folder folderData={folderData} isOpen={true} onToggle={setIsFolderOpen} onClickHandler={entryClicked}/>
                        </Suspense>
                    </Col>
                )}

                <Col md={isMobile ? 12 : (isFolderOpen ? 9 : 12)} lg={isMobile ? 12 : (isFolderOpen ? 10 : 12)} className={isMobile ? "ps-0 pe-0" : "section"}>
                    {!isFolderOpen && (
                        <Suspense fallback={<div className="loading">로딩 중...</div>}>
                            <Folder folderData={folderData} isOpen={false} onToggle={setIsFolderOpen} onClickHandler={entryClicked}/>
                        </Suspense>
                    )}
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
