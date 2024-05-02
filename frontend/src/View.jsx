import {useEffect, useState, useCallback, Suspense} from 'react';

import {getApiUrlPrefix} from './Common';

import './View.css';
import 'bootstrap/dist/css/bootstrap.min.css';
import {Container, Row, Col, Card, Alert} from 'react-bootstrap';

import {jsonGetReq} from './Common';
import Folder from './Folder.jsx';
import ViewSingle from "./ViewSingle.jsx";
import BookInfoView from "./BookInfoView.jsx";

export default function View() {
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

    const entryClicked = useCallback((selectedEntryId) => {
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
                    setBookInfo(book);
                    setViewUrl('/view/' + book['file_type'] + '/' + bookId + '/' + encodeURIComponent(book['file_path']));
                    setDownloadUrl(getApiUrlPrefix() + '/download/' + bookId);
                } else {
                    setErrorMessage(`can't find the selected book`);
                }
            } else {
                setErrorMessage(`can't find the selected category`);
            }
        }
    }, [folderData]);

    return (
        <Container id="view">
            <Row fluid="true">
                <Col md="3" lg="2" className="ps-0 pe-0 section">
                    <Suspense fallback={<div className="loading">로딩 중...</div>}>
                        <Folder folderData={folderData} onClickHandler={entryClicked}/>
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
