import {useEffect, useRef, useState} from "react";
import {useParams} from "react-router-dom";
import {getUrlPrefix, handleFetchErrors} from "./Common";

export default function ViewTXT(props) {
  const parentRef = useRef(null);
  const [height, setHeight] = useState(0);
  const [width, setWidth] = useState(0);

  const [errorMessage, setErrorMessage] = useState("");
  const [fileContent, setFileContent] = useState("");

  useEffect(() => {
    setHeight(parentRef.current.offsetHeight);
    setWidth(parentRef.current.offsetWidth);

    if (props && props.fileId) {
      const dirName = props.fileId.split('/')[0];
      const fileName = props.fileId.split('/')[1];
      const url = getUrlPrefix() + "/dirs/" + dirName + "/files/" + encodeURIComponent(fileName);
      fetch(url)
        .then(handleFetchErrors)
        .then((response) => {
          response.json()
            .then((result) => {
              if (result['status'] === 'success') {
                const html = result['result']['content'].replace(/\n/g, '<br/>');
                setFileContent(html);
                setErrorMessage(null);
              } else {
                setErrorMessage(result['error']);
                setFileContent('');
              }
            })
            .catch((error) => {
              setErrorMessage(error.message);
              setFileContent('');
            });
        })
        .catch((error) => {
          setErrorMessage(error.message);
          setFileContent('');
        });
    }

    return () => {
      //console.log("cleanup");
    };
  }, [props]);

  return (
    <div ref={parentRef}>
      <p className="text-left" dangerouslySetInnerHTML={{__html: fileContent}}></p>
    </div>
  );
};


