import {useEffect, useRef, useState} from "react";
import {getUrlPrefix} from "./Common";
import {Document, Page, pdfjs} from "react-pdf";

pdfjs.GlobalWorkerOptions.workerSrc = `//cdnjs.cloudflare.com/ajax/libs/pdf.js/${pdfjs.version}/pdf.worker.js`;

export default function ViewPDF(props) {
  const parentRef = useRef(null);
  const [height, setHeight] = useState(0);
  const [width, setWidth] = useState(0);

  const [errorMessage, setErrorMessage] = useState("");
  const [fileContent, setFileContent] = useState("");

  const [url, setUrl] = useState("");
  const [numPages, setNumPages] = useState(null);
  const [pageNumber, setPageNumber] = useState(1);

  useEffect(() => {
    console.log(`ViewPDF: useEffect(${props})`, props);
    setHeight(parentRef.current.offsetHeight);
    setWidth(parentRef.current.offsetWidth);

    if (props && props.fileId) {
      const dirName = props.fileId.split('/')[0];
      const fileName = props.fileId.split('/')[1];
      const url = getUrlPrefix() + "/download/dirs/" + dirName + "/files/" + encodeURIComponent(fileName);
      setUrl(url);
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


  return (
    <div ref={parentRef}>
      <Document
        file={url}
        onLoadSuccess={onDocumentLoadSuccess}>
        {Array.from(new Array(numPages), (el, index) => (
            <Page
              key={`page_${index + 1}`}
              pageNumber={index + 1}
              width={width}/>
          ),
        )}
      </Document>
    </div>
  );
};