import './Edit.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {useCallback, useEffect, useState, useRef, Suspense} from 'react';
import {useParams, useSearchParams, useOutletContext} from 'react-router-dom';

import {Alert, Button, Card, Col, Container, Form, InputGroup, Row} from 'react-bootstrap';
import {FontAwesomeIcon} from '@fortawesome/react-fontawesome';
import {faCheck, faTrash} from '@fortawesome/free-solid-svg-icons';

import {getApiUrlPrefix, jsonDeleteReq, jsonGetReq, jsonPutReq, ROOT_DIRECTORY} from './Common';
import Folder from './Folder';
import BookInfoView from './BookInfoView';
import Bookstore from './Bookstore';
import SimilarityDebug from './SimilarityDebug';
import EpubDiagnoseView from './EpubDiagnoseView';
import Actions from './Actions';
import SimilarBooks from './SimilarBooks';
import SearchResult from './SearchResult';
import ViewSingle from "./ViewSingle";
import {DateTime} from "luxon";
import {findCommonPrefix, buildFolderHierarchy, parseEntryId, findFolderInTree, updateFolderChildren, updateFolderInTree, determineNextEntryId, determinePrevEntryId} from './folderUtils';

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

export default function Edit() {
    const isMobile = useIsMobile();
    const params = useParams();
    const [searchParams] = useSearchParams();
    // 방법 B: /edit/bookId?category=... (우선) → 하위호환: /edit/category/bookId (폴백)
    const routeWildcard = params['*'] || '';
    const qCategory = searchParams.get('category');
    const routeCategory = qCategory || (routeWildcard ? parseEntryId(routeWildcard)?.category : undefined);
    const routeBookId = qCategory
        ? (/^\d+$/.test(routeWildcard) ? routeWildcard : undefined)
        : (routeWildcard ? parseEntryId(routeWildcard)?.bookId : undefined);
    const {searchResults, hasSearched, searchTotal, handleLoadMore, searchLoading} = useOutletContext();
    const [isFolderOpen, setIsFolderOpen] = useState(false);
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
    const [suggestedCategories, setSuggestedCategories] = useState({});
    const [searchTrigger, setSearchTrigger] = useState(0);

    // 비멱등 버튼 중복 실행 방지 가드
    const isProcessingRef = useRef(false);
    const [isProcessing, setIsProcessing] = useState(false);

    // entryClicked 함수의 최신 참조를 저장하는 ref (콜백에서 최신 함수 참조용)
    const entryClickedRef = useRef(null);
    // nextEntryId의 최신 값을 저장하는 ref (타이머 콜백에서 최신 값 참조용)
    const nextEntryIdRef = useRef(null);
    // prevEntryId의 최신 값을 저장하는 ref
    const prevEntryIdRef = useRef(null);

    // 메시지 자동 사라짐 (3초 후)
    useEffect(() => {
        if (successMessage) {
            const timer = setTimeout(() => setSuccessMessage(''), 3000);
            return () => clearTimeout(timer);
        }
    }, [successMessage]);

    useEffect(() => {
        if (errorMessage) {
            const timer = setTimeout(() => setErrorMessage(''), 5000);
            return () => clearTimeout(timer);
        }
    }, [errorMessage]);

    useEffect(() => {
        const categoryListUrl = '/categories';
        jsonGetReq(categoryListUrl, null, (categoryCounts) => {
            // categoryCounts: {"_epub": 5, "_pdf": 3, "_root": 2, ...}
            const categoryList = Object.keys(categoryCounts);

            // _root 카테고리(최상위 파일) 분리
            const hasRootFiles = categoryList.includes('_root');
            const nonEmptyCategories = categoryList.filter(c => c !== '_root');

            const commonPrefix = findCommonPrefix(nonEmptyCategories);

            // 2단계 계층 구조 생성
            let data = buildFolderHierarchy(
                nonEmptyCategories.sort((a, b) => a.localeCompare(b)),
                commonPrefix,
                categoryCounts
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
        // 처리 중에는 폴더 클릭을 무시하여 ref 덮어쓰기 방지
        if (isProcessingRef.current) return;

        // 선택된 항목 업데이트 (UI 동기화)
        setSelectedItems([selectedEntryId]);

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
            setSelectedEntryId(selectedEntryId);
            setOriginalBookInfo(book);
            const newBook = decomposeTitle(book);
            setBookInfo(newBook);
            setSearchTrigger(prev => prev + 1);
            setViewUrl('/viewer/' + book['file_type'] + '/' + bookId + '?path=' + encodeURIComponent(book['file_path']));
            setDownloadUrl(getApiUrlPrefix() + '/download/' + bookId);
            const otherCategoryList = categoryList
                .sort((a, b) => a.localeCompare(b))
                .filter(cat => cat !== '_root' && !cat.includes('/'));
            setOtherCategoryList(otherCategoryList);
            window.history.replaceState(null, '', `/edit/${bookId}?category=${encodeURIComponent(book['category'] || '_root')}`);

            // determine nextEntryId / prevEntryId
            const nextEntryId = determineNextEntryId(folderData, selectedEntryId);
            setNextEntryId(nextEntryId);
            nextEntryIdRef.current = nextEntryId;
            prevEntryIdRef.current = determinePrevEntryId(folderData, selectedEntryId);
        } else {
            // book entry
            const parsed = parseEntryId(selectedEntryId);
            if (!parsed) return;
            const category = parsed.category;
            const bookId = parsed.bookId;
            const folder = findFolderInTree(folderData, category);
            const booksInCategory = folder?.children;
            setSelectedEntryId(selectedEntryId);
            if (booksInCategory) {
                const book = booksInCategory.find(bookItem => bookItem.id === selectedEntryId)?.book;
                if (book) {
                    // save as original
                    setOriginalBookInfo(book);

                    // decompose file name to (author, title, extension)
                    const newBook = decomposeTitle(book);
                    setBookInfo(newBook);
                    setSearchTrigger(prev => prev + 1);
                    setViewUrl('/viewer/' + book['file_type'] + '/' + bookId + '?path=' + encodeURIComponent(book['file_path']));
                    setDownloadUrl(getApiUrlPrefix() + '/download/' + bookId);

                    // determine other category list
                    const otherCategoryList = categoryList
                        .sort((a, b) => a.localeCompare(b))
                        .filter(cat => cat !== category && cat !== '_root' && !cat.includes('/'))
                    setOtherCategoryList(otherCategoryList);
                    window.history.replaceState(null, '', `/edit/${bookId}?category=${encodeURIComponent(category)}`);

                    // determine nextEntryId / prevEntryId
                    const nextEntryId = determineNextEntryId(folderData, selectedEntryId);
                    setNextEntryId(nextEntryId);
                    nextEntryIdRef.current = nextEntryId;
                    prevEntryIdRef.current = determinePrevEntryId(folderData, selectedEntryId);
                } else {
                    setErrorMessage(`선택한 책을 찾을 수 없습니다. (ID: ${bookId})`);
                }
            } else {
                setErrorMessage(`선택한 카테고리를 찾을 수 없습니다. (${category})`);
            }
        }

    }, [folderData, categoryList, decomposeTitle]);

    // entryClicked ref 업데이트 (최신 함수 참조 유지)
    useEffect(() => {
        entryClickedRef.current = entryClicked;
    }, [entryClicked]);

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
                    // URL의 category 대신 API 응답의 실제 category 사용 (위조 방지)
                    const realCategory = book['category'] || routeCategory;
                    setSelectedEntryId(`${realCategory}/${routeBookId}`);
                    setOriginalBookInfo(book);
                    const newBook = decomposeTitle(book);
                    setBookInfo(newBook);
                    setSearchTrigger(prev => prev + 1);
                    setViewUrl('/viewer/' + book['file_type'] + '/' + routeBookId + '?path=' + encodeURIComponent(book['file_path']));
                    setDownloadUrl(getApiUrlPrefix() + '/download/' + routeBookId);
                    const otherCats = categoryList
                        .sort((a, b) => a.localeCompare(b))
                        .filter(cat => cat !== realCategory && cat !== '_root' && !cat.includes('/'));
                    setOtherCategoryList(otherCats);
                }, (error) => {
                    setErrorMessage(`책 정보를 불러올 수 없습니다. ${error}`);
                });
                return;
            }
            // load children if not yet
            if (!categoryItem.booksLoaded) {
                entryClicked(routeCategory);
            } else {
                entryClicked(`${routeCategory}/${routeBookId}`);
            }
        }
    }, [routeCategory, routeBookId, folderData, categoryList, decomposeTitle, entryClicked]);

    // 초기 로딩 시 첫 번째 파일 자동 선택
    useEffect(() => {
        // URL 파라미터가 없고, folderData가 로드되었고, 아직 선택된 책이 없는 경우
        if (!routeCategory && !routeBookId && folderData.length > 0 && !selectedEntryId) {
            // 첫 번째 파일(폴더가 아닌 항목) 찾기
            const firstFile = folderData.find(item => item.fileType !== 'folder' && item.book);
            if (firstFile) {
                setSelectedItems([firstFile.id]);
                entryClicked(firstFile.id);
            }
        }
    }, [routeCategory, routeBookId, folderData, selectedEntryId, entryClicked]);

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
            // 2단계 트리에서 폴더 검색
            const folder = findFolderInTree(folderData, newDirName);
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
            // 2단계 트리에서 폴더 내 파일 제거
            const entryId = dirName + '/' + fileName;
            newFolderData = updateFolderChildren(newFolderData, dirName, (children) =>
                children.filter(item => item.id !== entryId)
            );
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
            // 2단계 트리에서 폴더 내 파일 추가
            newFolderData = updateFolderChildren(newFolderData, newDirName, (children) => {
                const folders = children.filter(c => c.fileType === 'folder');
                const books = children.filter(c => c.fileType !== 'folder');
                const insertIdx = books.findIndex(b =>
                    newEntry.label.localeCompare(b.label) < 0
                );
                if (insertIdx === -1) {
                    books.push(newEntry);
                } else {
                    books.splice(insertIdx, 0, newEntry);
                }
                return [...folders, ...books];
            });
            console.log(`appendEntryToFolderData(): pushed ${newFileName} to ${newDirName}`);
        }
        return newFolderData;
    }, [bookInfo]);

    const updateFile = useCallback((dirName, fileName, newDirName, newFileName, force = false) => {
        if (isProcessingRef.current) return;

        const baseUrl = '/books/' + bookInfo['book_id'];
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
        const isSameFilePath = bookInfo['file_path'] === newFilePath;

        // 파일 경로가 변경되고 대상에 동일 이름이 이미 존재하면 confirm
        if (!force && !isSameFilePath && checkEntryExistence(folderData, newDirName, newFileName)) {
            const displayNewDirName = (newDirName === '' || newDirName === '_root') ? '최상위' : newDirName;
            if (!window.confirm(`"${displayNewDirName}/${newFileName}"이(가) 이미 존재합니다.\n기존 파일을 덮어쓰시겠습니까?`)) {
                return;
            }
            force = true;
        }

        // 가드 시작
        isProcessingRef.current = true;
        setIsProcessing(true);

        const requestUrl = force ? baseUrl + '?force=true' : baseUrl;
        const updatedTime = DateTime.now().toFormat('yyyy-MM-dd\'T\'HH:mm:ss.SSS');
        const payload = { ...bookInfo, category: categoryForBackend, file_path: newFilePath, updated_time: updatedTime };
        console.log(`updateFile: PUT ${requestUrl}, payload.title="${payload.title}", payload.author="${payload.author}", payload.file_path="${payload.file_path}"`);

        const handleSuccess = () => {
            isProcessingRef.current = false;
            setIsProcessing(false);

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

            setBookInfo(prev => ({
                ...prev,
                file_path: newFilePath,
                category: categoryForBackend
            }));
            setSearchTrigger(prev => prev + 1);

            let newFolderData = removeEntryFromFolderData(folderData, dirName, fileName);
            newFolderData = appendEntryToFolderData(newFolderData, newDirName, newFileName);
            setFolderData(newFolderData);

            if (dirName !== newDirName) {
                setSelectedCategory('');
                // 폴백 내비게이션: next → prev → 메시지
                if (nextEntryIdRef.current && entryClickedRef.current) {
                    entryClickedRef.current(nextEntryIdRef.current);
                } else if (prevEntryIdRef.current && entryClickedRef.current) {
                    entryClickedRef.current(prevEntryIdRef.current);
                } else {
                    setSuccessMessage(message + ' (마지막 책이었습니다.)');
                }
            }
        };

        jsonPutReq(requestUrl, payload, handleSuccess, (error) => {
            // 서버 측 충돌 감지: CONFLICT: 접두사로 시작하는 에러
            if (!force && typeof error === 'string' && error.startsWith('CONFLICT:')) {
                if (window.confirm(error.substring('CONFLICT:'.length) + '\n기존 파일을 덮어쓰시겠습니까?')) {
                    jsonPutReq(baseUrl + '?force=true', payload, handleSuccess, (retryError) => {
                        isProcessingRef.current = false;
                        setIsProcessing(false);
                        setErrorMessage(`책 변경에 실패했습니다. ${retryError}`);
                    });
                } else {
                    isProcessingRef.current = false;
                    setIsProcessing(false);
                }
            } else {
                isProcessingRef.current = false;
                setIsProcessing(false);
                console.error(`updateFile: PUT 실패 - ${error}`);
                setErrorMessage(`책 이름 변경에 실패했습니다. ${error}`);
            }
        });
    }, [bookInfo, folderData, checkEntryExistence, appendEntryToFolderData, removeEntryFromFolderData]);

    const changeButtonClicked = useCallback(() => {
        console.log(`changeButtonClicked: selectedEntryId=${selectedEntryId}, newFileName=${newFileName}`);
        if (selectedEntryId?.includes('/')) {
            const parsed = parseEntryId(selectedEntryId);
            if (parsed) {
                updateFile(parsed.category, parsed.bookId, parsed.category, newFileName);
            }
        }
    }, [updateFile, selectedEntryId, newFileName]);

    const moveToUpperButtonClicked = useCallback(() => {
        console.log(`move to upper directory as '${newFileName}'`);
        if (selectedEntryId?.includes('/')) {
            const parsed = parseEntryId(selectedEntryId);
            if (parsed) {
                updateFile(parsed.category, parsed.bookId, '_root', newFileName);
            }
        }
    }, [updateFile, selectedEntryId, newFileName]);

    const selectDirectoryButtonClicked = useCallback((e, props) => {
        setSelectedCategory(props);
    }, []);

    const moveToDirectoryButtonClicked = useCallback(() => {
        console.log(`move to '${selectedCategory}' as '${newFileName}'`);
        if (selectedEntryId?.includes('/')) {
            const parsed = parseEntryId(selectedEntryId);
            if (parsed) {
                updateFile(parsed.category, parsed.bookId, selectedCategory, newFileName);
            }
        }
    }, [updateFile, selectedEntryId, selectedCategory, newFileName]);

    const deleteButtonClicked = useCallback(() => {
        if (isProcessingRef.current) return;
        console.log(`deleteButtonClicked: entryId=${selectedEntryId}`);
        if (selectedEntryId?.includes('/')) {
            const parsed = parseEntryId(selectedEntryId);
            if (!parsed) return;
            if (!window.confirm(`"${newFileName}"을(를) 삭제하시겠습니까?`)) return;

            // 가드 시작
            isProcessingRef.current = true;
            setIsProcessing(true);

            const dirName = parsed.category;
            const bookId = parsed.bookId;
            const deleteUrl = '/books/' + bookId;
            console.log(deleteUrl);
            jsonDeleteReq(deleteUrl, null, (response) => {
                isProcessingRef.current = false;
                setIsProcessing(false);

                const displayDirName = (dirName === '' || dirName === '_root') ? '최상위' : dirName;
                // warning이 있으면 경고 메시지와 함께 표시
                let message;
                if (response?.warning) {
                    message = `"${displayDirName}/${newFileName}"이(가) 삭제되었습니다. (경고: ${response.warning})`;
                } else {
                    message = `"${displayDirName}/${newFileName}"이(가) 삭제되었습니다.`;
                }
                setSuccessMessage(message);
                setErrorMessage('');

                const newFolderData = removeEntryFromFolderData(folderData, dirName, bookId);
                setFolderData(newFolderData);

                // 폴백 내비게이션: next → prev → 메시지
                if (nextEntryIdRef.current && entryClickedRef.current) {
                    console.log(`deleteButtonClicked(): nextEntryId=${nextEntryIdRef.current}`);
                    entryClickedRef.current(nextEntryIdRef.current);
                } else if (prevEntryIdRef.current && entryClickedRef.current) {
                    console.log(`deleteButtonClicked(): prevEntryId=${prevEntryIdRef.current}`);
                    entryClickedRef.current(prevEntryIdRef.current);
                } else {
                    setSuccessMessage(message + ' (마지막 책이었습니다.)');
                }
            }, (error) => {
                isProcessingRef.current = false;
                setIsProcessing(false);
                setErrorMessage(`책 삭제에 실패했습니다. ${error}`);
            });
        }
    }, [selectedEntryId, folderData, removeEntryFromFolderData, newFileName]);

    const toNextEntryButtonClicked = useCallback(() => {
        if (isProcessingRef.current) return;
        console.log(`toNextEntryButtonClicked: nextEntryId=${nextEntryId}`);
        if (nextEntryId) {
            setSelectedItems([nextEntryId]);
            entryClicked(nextEntryId);
        } else {
            setErrorMessage('마지막 책입니다.');
        }
    }, [nextEntryId, entryClicked]);

    // 모바일에서는 directory-menu 클래스를 제거하여 고정 높이 스타일 방지
    const directoryClassName = isMobile
        ? "ps-0 pe-0"
        : "ps-0 pe-0 section directory-menu";

    return (
        <Container id="edit">
            <Row fluid="true">
                {isFolderOpen && (
                    <Col md={isMobile ? 12 : 3} lg={isMobile ? 12 : 2} className={directoryClassName}>
                        <Suspense fallback={<div className="loading">로딩 중...</div>}>
                            <Folder folderData={folderData} selectedItems={selectedItems} isOpen={true} onToggle={setIsFolderOpen} onClickHandler={entryClicked}/>
                        </Suspense>
                    </Col>
                )}

                <Col md={isMobile ? 12 : (isFolderOpen ? 9 : 12)} lg={isMobile ? 12 : (isFolderOpen ? 10 : 12)} className={isMobile ? "ps-0 pe-0" : "section"}>
                    {!isFolderOpen && (
                        <Suspense fallback={<div className="loading">로딩 중...</div>}>
                            <Folder folderData={folderData} selectedItems={selectedItems} isOpen={false} onToggle={setIsFolderOpen} onClickHandler={entryClicked}/>
                        </Suspense>
                    )}
                    {hasSearched &&
                        <SearchResult
                            results={searchResults}
                            showEditButton={true}
                            onLoadMore={handleLoadMore}
                            hasMore={searchResults.length < searchTotal}
                            loading={searchLoading}
                        />
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
                                                                <Button variant="outline-success" className="btn-xs" onClick={changeButtonClicked} disabled={!selectedEntryId || isProcessing}>
                                                                    변경
                                                                    <FontAwesomeIcon icon={faCheck}/>
                                                                </Button>
                                                                <Button variant="outline-danger" className="btn-xs" onClick={deleteButtonClicked} disabled={!selectedEntryId || isProcessing}>
                                                                    삭제
                                                                    <FontAwesomeIcon icon={faTrash}/>
                                                                </Button>
                                                            </InputGroup>
                                                        </Col>
                                                    </Row>

                                                    <Row className="button_group">
                                                        <Col>
                                                            <Actions selectedEntryId={selectedEntryId} selectedCategory={selectedCategory} otherCategoryList={otherCategoryList} newFileName={newFileName} moveToUpperButtonClicked={moveToUpperButtonClicked} moveToDirectoryButtonClicked={moveToDirectoryButtonClicked} selectDirectoryButtonClicked={selectDirectoryButtonClicked} toNextEntryClicked={toNextEntryButtonClicked} suggestedCategories={suggestedCategories} isProcessing={isProcessing}/>
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
                                    <Bookstore bookInfo={bookInfo} searchTrigger={searchTrigger} onCategoriesFound={setSuggestedCategories}/>
                                    <SimilarityDebug suggestedCategories={suggestedCategories} categoryList={otherCategoryList}/>
                                    <EpubDiagnoseView bookId={bookInfo['book_id']} fileType={bookInfo['file_type']}/>
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
                                        preview={true}
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
