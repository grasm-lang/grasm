;;; SBCL's READ-FROM-STRING on one token per line: <base> <float format>
;;; <token as hex>. Answers I:<integer>, R:<num>/<den>, F:<8 hex digits of a
;;; single-float>, D:<16 hex digits of a double-float>, S:<symbol name as
;;; hex>, or E:<eof|impossible|reader|other>.
(defun unhex (h)
  (if (string= h "-")
      ""
      (let ((octets (make-array (floor (length h) 2) :element-type '(unsigned-byte 8))))
        (dotimes (i (length octets))
          (setf (aref octets i) (parse-integer h :start (* 2 i) :end (+ 2 (* 2 i)) :radix 16)))
        (sb-ext:octets-to-string octets :external-format :utf-8))))

(defun hex (s)
  (let ((octets (sb-ext:string-to-octets s :external-format :utf-8)))
    (if (zerop (length octets))
        "-"
        (with-output-to-string (o)
          (loop for b across octets do (format o "~(~2,'0x~)" b))))))

(defun split (line)
  (loop with start = 0
        for pos = (position #\Space line :start start)
        collect (subseq line start pos)
        while pos
        do (setf start (1+ pos))))

(defun describe-value (v)
  (typecase v
    (integer (format nil "I:~D" v))
    (ratio (format nil "R:~D/~D" (numerator v) (denominator v)))
    (single-float (format nil "F:~(~8,'0x~)" (ldb (byte 32 0) (sb-kernel:single-float-bits v))))
    (double-float (format nil "D:~(~16,'0x~)" (logior (ash (ldb (byte 32 0) (sb-kernel:double-float-high-bits v)) 32)
                                                      (sb-kernel:double-float-low-bits v))))
    (symbol (format nil "S:~A" (hex (symbol-name v))))
    (t "O")))

(loop for line = (read-line *standard-input* nil)
      while line
      do (let* ((toks (split line))
                (*read-base* (parse-integer (first toks)))
                (*read-default-float-format* (intern (string-upcase (second toks)) "COMMON-LISP"))
                (text (unhex (third toks))))
           (write-line
            (handler-case (describe-value (read-from-string text))
              (end-of-file () "E:eof")
              (sb-kernel:reader-impossible-number-error () "E:impossible")
              (reader-error () "E:reader")
              (error () "E:other")))))
