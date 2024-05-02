import {useEffect, useRef, useState, Suspense} from "react";
import PropTypes from 'prop-types';
import {getApiUrlPrefix} from "./Common";
import {Document, Page, pdfjs} from "react-pdf";

pdfjs.GlobalWorkerOptions.workerSrc = `//cdnjs.cloudflare.com/ajax/libs/pdf.js/${pdfjs.version}/pdf.worker.js`;

export default function ViewPDF(props) {
    const parentRef = useRef(null);
    const [width, setWidth] = useState(0);

    const [url, setUrl] = useState("");
    const [numPages, setNumPages] = useState(null);
    const [, setPageNumber] = useState(1);
    const [pageCount, setPageCount] = useState(0);

    useEffect(() => {
        console.log(`ViewPDF: useEffect(${props})`, props);
        setWidth(parentRef.current.offsetWidth);

        const url = getApiUrlPrefix() + '/download/' + props.bookId;
        console.log(url);
        setUrl(url);
        setPageCount(props.pageCount);

        return () => {
            setUrl('');
            setNumPages(0);
            setPageNumber(1);
        };
    }, [props]);

    function onDocumentLoadSuccess({numPages}) {
        setNumPages(numPages);
    }

    /*
    function changePage(offset) {
        setPageNumber(prevPageNumber => prevPageNumber + offset);
    }
    */

    return (
        <div ref={parentRef}>
            <Suspense fallback={<div className="loading">로딩 중...</div>}>
                <Document
                    file={url}
                    onLoadSuccess={onDocumentLoadSuccess}>
                    {Array.from(new Array(numPages), (el, index) => {
                            if (pageCount && index + 1 > pageCount) {
                                return;
                            }
                            return (
                                <Page
                                    key={`page_${index + 1}`}
                                    pageNumber={index + 1}
                                    width={width}/>
                            )
                        },
                    )}
                </Document>
            </Suspense>
        </div>
    );
}

ViewPDF.propTypes = {
    bookId: PropTypes.number.isRequired,
    pageCount: PropTypes.number.isRequired
};