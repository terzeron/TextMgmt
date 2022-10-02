import {useEffect, useRef, useState} from "react";
import {getUrlPrefix, handleFetchErrors} from "./Common";

export default function ViewTXT(props) {
  const parentRef = useRef(null);
  const ref = useRef(null);
  const [height, setHeight] = useState(0);
  const [width, setWidth] = useState(0);

  const [errorMessage, setErrorMessage] = useState("");
  const [fileContent, setFileContent] = useState("");

  const [url, setUrl] = useState("");

  useEffect(() => {
    console.log(`ViewHTML: useEffect(${props})`, props);
    setHeight(parentRef.current.offsetHeight);
    setWidth(parentRef.current.offsetWidth);

    if (props && props.fileId) {
      const dirName = props.fileId.split('/')[0];
      const fileName = props.fileId.split('/')[1];
      const url = getUrlPrefix() + "/download/dirs/" + dirName + "/files/" + encodeURIComponent(fileName);
      console.log(url);
      setUrl(url);
      console.log(ref.current);
    }

    return () => {
      //console.log("cleanup");
    };
  }, [props]);

  return (
    <div ref={parentRef}>
      <iframe src={url} ref={ref}/>
    </div>
  );
};


