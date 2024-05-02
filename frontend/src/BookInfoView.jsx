import {useEffect, useState, Suspense} from 'react';
import PropTypes from 'prop-types';

import './Edit.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {Button, Card, Col, Form, InputGroup, Row} from 'react-bootstrap';
import {FontAwesomeIcon} from '@fortawesome/react-fontawesome';
import {faClockRotateLeft, faCut, faRotate} from '@fortawesome/free-solid-svg-icons';


export default function BookInfoView(props) {
    const [, setBookId] = useState('');
    const [, setCategory] = useState('');
    const [title, setTitle] = useState('');
    const [author, setAuthor] = useState('');
    const [fileType, setFileType] = useState('');
    const [filePath, setFilePath] = useState('');
    const [fileSize, setFileSize] = useState(0);
    const [isEditEnabled, setIsEditEnabled] = useState(false);

    useEffect(() => {
        const bookInfo = props.bookInfo
        setBookId(bookInfo['book_id']);
        setCategory(bookInfo['category']);
        setTitle(bookInfo['title']);
        setAuthor(bookInfo['author']);
        setFileType(bookInfo['file_type']);
        setFilePath(bookInfo['file_path']);
        setFileSize(bookInfo['file_size']);
        setIsEditEnabled(props.isEditEnabled);
    }, [props]);

    return (
        <Card>
            <Card.Header>
                책 정보
            </Card.Header>
            <Suspense fallback={<div className="loading">로딩 중...</div>}>
                <Card.Body>
                    <Row>
                        <Col>
                            <Form.Control value={filePath} readOnly disabled/>
                        </Col>
                    </Row>

                    <Row>
                        <Col xs="6">
                            <InputGroup>
                                <InputGroup.Text>파일 종류</InputGroup.Text>
                                <Form.Control value={fileType} readOnly disabled/>
                            </InputGroup>
                        </Col>
                        <Col xs="6">
                            <InputGroup>
                                <InputGroup.Text>파일 크기</InputGroup.Text>
                                <Form.Control value={fileSize.toLocaleString()} readOnly disabled/>
                            </InputGroup>
                        </Col>
                    </Row>

                    {
                        isEditEnabled &&
                        <Row>
                            <InputGroup>
                                <InputGroup.Text>저자</InputGroup.Text>
                                <Form.Control value={author} onChange={(e) => {
                                    props.onAuthorChange(e)
                                }}/>
                                <Button variant="outline-secondary" size="sm" onClick={(e) => {
                                    props.onCutAuthorButtonClick(e)
                                }}>
                                    분할
                                    <FontAwesomeIcon icon={faCut}/>
                                </Button>
                                <Button variant="outline-secondary" size="sm" onClick={props.onExchangeButtonClick}>
                                    교환
                                    <FontAwesomeIcon icon={faRotate}/>
                                </Button>
                            </InputGroup>
                        </Row>
                    }

                    {
                        isEditEnabled &&
                        <Row>
                            <InputGroup>
                                <InputGroup.Text>제목</InputGroup.Text>
                                <Form.Control value={title} onChange={(e) => props.onTitleChange(e)}/>
                                <Button variant="outline-secondary" size="sm" onClick={(e) => {
                                    props.onCutTitleButtonClick(e)
                                }}>
                                    분할
                                    <FontAwesomeIcon icon={faCut}/>
                                </Button>
                                <Button variant="outline-secondary" size="sm" onClick={(e) => {
                                    props.onResetButtonClick(e)
                                }}>
                                    복원
                                    <FontAwesomeIcon icon={faClockRotateLeft}/>
                                </Button>
                            </InputGroup>
                        </Row>
                    }
                </Card.Body>
            </Suspense>
        </Card>
    );
}

BookInfoView.propTypes = {
    bookInfo: PropTypes.object.isRequired,
    isEditEnabled: PropTypes.bool,
    onTitleChange: PropTypes.func,
    onAuthorChange: PropTypes.func,
    onCutTitleButtonClick: PropTypes.func,
    onCutAuthorButtonClick: PropTypes.func,
    onExchangeButtonClick: PropTypes.func,
    onResetButtonClick: PropTypes.func
};