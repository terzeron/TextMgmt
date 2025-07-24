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
    const {entryId, fileType, filePath: path} = useParams();
    const [bookId, setBookId] = useState(0);
    const [filePath, setFilePath] = useState('');
    const [currentFileType, setCurrentFileType] = useState('');
    const [lineCount, setLineCount] = useState(0);
    const [pageCount, setPageCount] = useState(0);

    useEffect(() => {
        if (entryId && fileType && path) {
            // as single page
            setBookId(entryId);
            setCurrentFileType(fileType);
            setFilePath(decodeURIComponent(path));
        } else if (props.bookId) {
            // as sub-component
            setBookId(props.bookId);
            setFilePath(props.filePath);
            setCurrentFileType(props.fileType);
            setLineCount(props.lineCount);
            setPageCount(props.pageCount);
        }
        return () => {
            setBookId(0);
        };
    }, [props, entryId, fileType, path]);

    const componentMap = {
        'pdf': <ViewPDF bookId={bookId} pageCount={pageCount} filePath={filePath}/>,
        'epub': <ViewEPUB bookId={bookId} filePath={filePath} />,
        'doc': <ViewDOC bookId={bookId} filePath={filePath} />,
        'docx': <ViewDOC bookId={bookId} filePath={filePath} />,
        'txt': <ViewTXT bookId={bookId} lineCount={lineCount} filePath={filePath} />,
        'html': <ViewHTML bookId={bookId} filePath={filePath} />,
        'rtf': <ViewRTF bookId={bookId} filePath={filePath} />,
        'jpg': <ViewImage bookId={bookId} filePath={filePath} />,
        'gif': <ViewImage bookId={bookId} filePath={filePath} />,
        'png': <ViewImage bookId={bookId} filePath={filePath} />
    };
    const renderComponent = componentMap[currentFileType];

    return (
        <Card>
            <Card.Header>
                책 보기
                <span>
                    {
                        props.viewUrl &&
                        <a href={props.viewUrl} target="_blank" rel="noreferrer">
                            <Button variant="outline-primary" size="sm" disabled={!props.viewUrl} className="float-end">새 창에서 전체 보기</Button>
                        </a>
                    }
                    {
                        props.downloadUrl &&
                        <a href={props.downloadUrl} target="_blank" rel="noreferrer">
                            <Button variant="outline-primary" size="sm" disabled={!props.downloadUrl} className="float-end">다운로드</Button>
                        </a>
                    }
                </span>
            </Card.Header>
            <Card.Body>
                {
                    bookId && (
                        <>
                            { bookId && renderComponent }
                        </>
                    )
                }
                {
                    !bookId && <div>책이 선택되지 않았습니다.</div>
                }
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