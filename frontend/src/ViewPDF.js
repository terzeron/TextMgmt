import {useEffect, useRef, useState} from "react";
import {getUrlPrefix} from "./Common";
import {Document, Page, pdfjs} from "react-pdf";

pdfjs.GlobalWorkerOptions.workerSrc = `//cdnjs.cloudflare.com/ajax/libs/pdf.js/${pdfjs.version}/pdf.worker.js`;

export default function ViewPDF(props) {
  const [loading, setLoading] = useState(false);
  const parentRef = useRef(null);
  const [width, setWidth] = useState(0);

  const [errorMessage, setErrorMessage] = useState("");

  const [url, setUrl] = useState("");
  const [numPages, setNumPages] = useState(null);
  const [pageNumber, setPageNumber] = useState(1);

  useEffect(() => {
    console.log(`ViewPDF: useEffect(${props})`, props);
    setWidth(parentRef.current.offsetWidth);

    if (props && props.entryId) {
      setLoading(true);
      const dirName = props.entryId.split('/')[0];
      const fileName = props.entryId.split('/')[1];
      const url = getUrlPrefix() + "/download/dirs/" + dirName + "/files/" + encodeURIComponent(fileName);
      setUrl(url);
      setLoading(false);
    }

    return () => {
      //console.log("cleanup");
    };
  }, [props]);

  function onDocumentLoadSuccess({numPages}) {
    setNumPages(numPages);
  }

  function changePage(offset) {
    setPageNumber(prevPageNumber => prevPageNumber + offset);
  }

  if (loading) return <div>로딩 중...</div>;
  if (errorMessage) return <div>{errorMessage}</div>;
  return (
    <div ref={parentRef}>
      <Document
        file={url}
        onLoadSuccess={onDocumentLoadSuccess}>
        {Array.from(new Array(numPages), (el, index) => {
            if (index > 5) {
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
    </div>
  );
};