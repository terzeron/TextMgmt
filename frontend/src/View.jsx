import {useEffect, useState, useCallback, Suspense} from 'react';

import {getApiUrlPrefix} from './Common';

import './View.css';
import 'bootstrap/dist/css/bootstrap.min.css';
import {Container, Row, Col, Card, Alert} from 'react-bootstrap';

import {jsonGetReq} from './Common';
import Folder from './Folder.jsx';
import ViewSingle from "./ViewSingle.jsx";
import BookInfoView from "./BookInfoView.jsx";
import useSearchStore from "./stores/searchStore.js";
import SearchResult from "./SearchResult.jsx";

export default function View() {
    const {searchResult, isLoading, error} = useSearchStore();
    const [errorMessage, setErrorMessage] = useState('');
    const [successMessage, setSuccessMessage] = useState('');
    const [selectedEntryId, setSelectedEntryId] = useState('');
    const [folderData, setFolderData] = useState([]);
    const [bookInfo, setBookInfo] = useState({});
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
        }, (error) => {
            setErrorMessage(`can't load directory data, ${error}`);
        });

        return () => {
            setErrorMessage('');
            setSuccessMessage('');
            setSelectedEntryId('');
            setFolderData([]);
            setBookInfo({});
            setViewUrl('');
            setDownloadUrl('');
        }
    }, []);

    const selectBook = useCallback((book) => {
        if (!book || !book.book_id) {
            setErrorMessage('Invalid book selected');
            return;
        }

        const { book_id, category, file_path, file_type } = book;

        setBookInfo(book);
        setViewUrl('/view/' + file_type + '/' + book_id + '/' + encodeURIComponent(file_path));
        setDownloadUrl(getApiUrlPrefix() + '/download/' + book_id);
        setSelectedEntryId(`${category}/${book_id}`);
    }, []);

    const entryClicked = useCallback((entry) => {
        if (typeof entry === 'string') {
            const selectedEntryId = entry;
            const isCategoryClick = !selectedEntryId.includes('/');
            if (isCategoryClick) {
                const categoryId = selectedEntryId;
                const isChildrenLoaded = folderData.find(item => item.id === categoryId)?.children;
                if (!isChildrenLoaded) {
                    const booksInCategoryUrl = '/categories/' + categoryId;
                    jsonGetReq(booksInCategoryUrl, null, (bookList) => {
                        setFolderData(currentFolderData => currentFolderData.map(item =>
                            item.id === categoryId
                                ? { ...item, children: bookList.sort((a, b) => a['title'].localeCompare(b['title'])).map(b => ({
                                        id: `${item.id}/${b.book_id}`,
                                        label: `${b.title}.${b.file_type}`,
                                        fileType: b.file_type,
                                        children: [],
                                        book: b,
                                    }))}
                                : item
                        ));
                    });
                }
            } else { // This is a book click from folder tree
                const category = selectedEntryId.split('/')[0];
                const book = folderData.find(c => c.id === category)?.children?.find(b => b.id === selectedEntryId)?.book;
                if (book) {
                    selectBook(book);
                }
            }
        } else {
            const book = entry;
            selectBook(book);

            const categoryId = book.category;
            const isCategoryLoaded = folderData.find(item => item.id === categoryId)?.children;
            if (!isCategoryLoaded) {
                const booksInCategoryUrl = '/categories/' + categoryId;
                jsonGetReq(booksInCategoryUrl, null, (bookList) => {
                    setFolderData(currentFolderData => currentFolderData.map(item =>
                        item.id === categoryId
                            ? { ...item, children: bookList.sort((a, b) => a['title'].localeCompare(b['title'])).map(b => ({
                                    id: `${item.id}/${b.book_id}`,
                                    label: `${b.title}.${b.file_type}`,
                                    fileType: b.file_type,
                                    children: [],
                                    book: b,
                                }))}
                            : item
                    ));
                });
            }
        }
    }, [folderData, selectBook]);

    return (
        <Container id="view">
            <Row fluid="true">
                <Col md="3" lg="2" className="ps-0 pe-0 section">
                    <Suspense fallback={<div className="loading">로딩 중...</div>}>
                        <Folder folderData={folderData} selectedItems={[selectedEntryId]} onClickHandler={entryClicked}/>
                    </Suspense>
                </Col>

                <Col md="9" lg="10" className="section">
                    <Row id="top_panel">
                        <Col id="left_panel" md="6" lg="5" className="ps-0 pe-0">
                            {
                                !bookInfo['book_id'] &&
                                <Card.Body>책이 선택되지 않았습니다.</Card.Body>
                            }
                            {
                                bookInfo['book_id'] &&
                                <>
                                    <BookInfoView bookInfo={bookInfo} isEditEnabled={false}/>
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
                                </>
                            }
                        </Col>
                        <Col id="right_panel" md="6" lg="7" className="ps-0 pe-0">
                            <SearchResult results={searchResult} isLoading={isLoading} error={error} entryClicked={entryClicked}/>
                        </Col>
                    </Row>
                    {
                        bookInfo['book_id'] &&
                        <Row id="bottom_panel">
                            <Col id="right_panel" className="ps-0 pe-0">
                                {
                                    bookInfo['book_id'] &&
                                    <ViewSingle key={bookInfo['book_id']} bookId={bookInfo['book_id']} filePath={bookInfo['file_path']} fileType={bookInfo['file_type']} viewUrl={viewUrl} downloadUrl={downloadUrl} lineCount={100} pageCount={10}/>
                                }
                            </Col>
                        </Row>
                    }
                </Col>
            </Row>
        </Container>
    );
}
