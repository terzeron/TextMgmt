import {useEffect, useState} from "react";
import {useParams} from "react-router-dom";
import PropTypes from 'prop-types';

import './ViewSingle.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import ViewPDF from "./ViewPDF";
import ViewEPUB from "./ViewEPUB";
import ViewDOC from "./ViewDOC";
import ViewTXT from "./ViewTXT";
import ViewHTML from './ViewHTML';
import ViewRTF from './ViewRTF';
import ViewImage from "./ViewImage";
import {Button, Card} from "react-bootstrap";

export default function ViewSingle(props) {
    const { entryId, fileType: paramFileType, filePath: paramFilePath } = useParams();
    const standalone = Boolean(entryId && paramFileType && paramFilePath);
    const [bookId, setBookId] = useState(0);
    const [filePath, setFilePath] = useState('');
    const [fileType, setFileType] = useState('');
    const [lineCount, setLineCount] = useState(0);
    const [pageCount, setPageCount] = useState(0);

    useEffect(() => {
        if (entryId && paramFileType && paramFilePath) {
            // standalone page via route
            setBookId(Number(entryId));
            setFileType(paramFileType);
            setFilePath(paramFilePath);
        } else if (props.bookId) {
            // nested component
            setBookId(props.bookId);
            setFileType(props.fileType);
            setFilePath(props.filePath);
            setLineCount(props.lineCount);
            setPageCount(props.pageCount);
        }
        return () => {
            setBookId(0);
        };
    }, [props, entryId, paramFileType, paramFilePath]);

    const componentMap = {
        'pdf': <ViewPDF bookId={bookId} pageCount={pageCount} />,
        'epub': <ViewEPUB bookId={bookId} filePath={filePath} />,
        'doc': <ViewDOC bookId={bookId} />,
        'docx': <ViewDOC bookId={bookId} lineCount={lineCount} />,
        'txt': <ViewTXT bookId={bookId} lineCount={lineCount} />,
        'html': <ViewHTML bookId={bookId} />,
        'rtf': <ViewRTF bookId={bookId} />,
        'jpg': <ViewImage bookId={bookId} />,
        'gif': <ViewImage bookId={bookId} />,
        'png': <ViewImage bookId={bookId} />
    };
    const renderComponent = componentMap[fileType];

    return (
        <Card>
            {!standalone && (
                <Card.Header>
                    책 보기
                    <span>
                        {props.viewUrl && (
                            <a href={props.viewUrl} target="_blank" rel="noreferrer">
                                <Button variant="outline-primary" size="sm" disabled={!props.viewUrl} className="float-end">
                                    새 창에서 전체 보기
                                </Button>
                            </a>
                        )}
                        {props.downloadUrl && (
                            <a href={props.downloadUrl} target="_blank" rel="noreferrer">
                                <Button variant="outline-primary" size="sm" disabled={!props.downloadUrl} className="float-end">
                                    다운로드
                                </Button>
                            </a>
                        )}
                    </span>
                </Card.Header>
            )}
            <Card.Body>
                {bookId ? (
                    renderComponent
                ) : (
                    <div>책이 선택되지 않았습니다.</div>
                )}
            </Card.Body>
        </Card>
    )
}

ViewSingle.propTypes = {
    bookId: PropTypes.number,
    filePath: PropTypes.string,
    fileType: PropTypes.string,
    viewUrl: PropTypes.string,
    downloadUrl: PropTypes.string,
    lineCount: PropTypes.number,
    pageCount: PropTypes.number
}