import './ViewSingle.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {useEffect, useRef, useState} from "react";
import {useParams} from "react-router-dom";

import ViewPDF from "./ViewPDF";
import ViewEPUB from "./ViewEPUB";
import ViewTXT from "./ViewTXT";
import ViewHTML from './ViewHTML';

export default function ViewSingle(props) {
  const { dirName, fileName } = useParams();
  const [fileId, setFileId] = useState("");
  const [errorMessage, setErrorMessage] = useState("");

  useEffect(() => {
    console.log(`ViewSingle() props=`, props);
    console.log(`ViewSingle(dirName=${dirName}, fileName=${fileName})`);

    if (props.fileId) {
      setFileId(props.fileId);
    }
    if (dirName && fileName) {
      setFileId(dirName + "/" + fileName);
    }

    return () => {
      //console.log("cleanup");
    }
  }, [props, dirName, fileName]);

  return (
    <div>
      {
        fileId && fileId.endsWith(".pdf") && <ViewPDF fileId={fileId}/>
      }
      {
        fileId && fileId.endsWith(".epub") && <ViewEPUB fileId={fileId}/>
      }
      {
        fileId && fileId.endsWith(".txt") && <ViewTXT fileId={fileId}/>
      }
      {
        fileId && fileId.endsWith(".html") && <ViewHTML fileId={fileId}/>
      }
    </div>
  );
}
