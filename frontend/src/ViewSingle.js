import './ViewSingle.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {useEffect, useRef, useState} from "react";
import {useParams} from "react-router-dom";

import ViewPDF from "./ViewPDF";
import ViewEPUB from "./ViewEPUB";
import ViewTXT from "./ViewTXT";
import ViewHTML from './ViewHTML';

export default function ViewSingle(props) {
  const {dirName, fileName} = useParams();
  const [entryId, setEntryId] = useState("");
  const [size, setSize] = useState(0);
  const [errorMessage, setErrorMessage] = useState("");

  useEffect(() => {
    if (props) {
      console.log(`ViewSingle: useEffect() props=`, props);
    } else {
      console.log(`ViewSingle: useEffect() dirName=${dirName}, fileName=${fileName})`);
    }

    if (props.entryId) {
      setEntryId(props.entryId);
      setSize(10 * 1024);
    }
    if (dirName && fileName) {
      setEntryId(dirName + "/" + fileName);
    }

    return () => {
      console.log("cleanup ViewSingle");
      setEntryId("");
    }
  }, [props]);

  return (
    <div>
      {
        entryId && entryId.endsWith(".pdf") && <ViewPDF entryId={entryId}/>
      }
      {
        entryId && entryId.endsWith(".epub") && <ViewEPUB entryId={entryId}/>
      }
      {
        entryId && entryId.endsWith(".txt") && <ViewTXT entryId={entryId} size={size}/>
      }
      {
        entryId && entryId.endsWith(".html") && <ViewHTML entryId={entryId}/>
      }
    </div>
  );
}
