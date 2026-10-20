;;; The oracle for packages/lispnumbers: SBCL 2.6's ~R family. A line is
;;; "lispnumbers c N", "o N", "r N", "or N", or "x N BASE MINCOL PAD COMMA
;;; INTERVAL COLON AT" (PAD and COMMA as hex of one character); the answer
;;; is the printed text, or E: and the error's message.
(defun unhex (h)
  (code-char (parse-integer h :radix 16)))

(defun split (line)
  (let ((parts '()) (start 0))
    (loop for i from 0 to (length line)
          do (when (or (= i (length line)) (char= (char line i) #\Space))
               (when (> i start) (push (subseq line start i) parts))
               (setf start (1+ i))))
    (nreverse parts)))

(defun answer (f)
  (let ((op (second f)) (n (parse-integer (third f))))
    (handler-case
        (cond ((string= op "c") (format nil "~R" n))
              ((string= op "o") (format nil "~:R" n))
              ((string= op "r") (format nil "~@R" n))
              ((string= op "or") (format nil "~:@R" n))
              (t (destructuring-bind (base mincol pad comma interval colon at) (cdddr f)
                   (format nil (format nil "~~~A,~A,'~A,'~A,~A~A~AR" base mincol (unhex pad) (unhex comma) interval
                                       (if (string= colon "1") ":" "") (if (string= at "1") "@" ""))
                           n))))
      (error (e) (format nil "E:~A" e)))))

(loop for line = (read-line *standard-input* nil)
      while line
      do (when (> (length line) 0)
           (write-line (answer (split line)))))
