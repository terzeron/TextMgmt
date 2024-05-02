import {useEffect, useState} from "react";
import PropTypes from 'prop-types';
import {getApiUrlPrefix} from "./Common";

export default function ViewImage(props) {
  const [url, setUrl] = useState("");

  useEffect(() => {
    console.log(`ViewImage: useEffect(${props})`, props);

    if (props && props.bookId) {
      const url = getApiUrlPrefix() + '/download/' + props.bookId;
      console.log(url);
      setUrl(url);
    }

    return () => {
      setUrl('')
    };
  }, [props]);

  return (
    <img src={url} alt='book image'/>
  );
}

ViewImage.propTypes = {
    bookId: PropTypes.number.isRequired,
}



