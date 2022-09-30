import {useEffect, useRef, useState} from "react";
import {getUrlPrefix} from "./Common";
import {ReactReader} from "react-reader"
import {useParams} from "react-router-dom";

export default function ViewEPUB(props) {
  const parentRef = useRef(null);
  const [height, setHeight] = useState(0);
  const [width, setWidth] = useState(0);

  const [errorMessage, setErrorMessage] = useState("");
  const [fileContent, setFileContent] = useState("");

  const [url, setUrl] = useState("");
  const [location, setLocation] = useState(null)

  const locationChanged = (epubcifi) => {
    // epubcifi is a internal string used by epubjs to point to a location in an epub. It looks like this: epubcfi(/6/6[titlepage]!/4/2/12[pgepubid00003]/3:0)
    setLocation(epubcifi)
  }

  useEffect(() => {
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

  return (
    <div style={{height: "100vh"}} ref={parentRef}>
      <ReactReader
        location={location}
        locationChanged={locationChanged}
        url={url}/>
    </div>
  );
};