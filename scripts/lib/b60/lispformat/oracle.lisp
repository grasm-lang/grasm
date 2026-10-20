;;; SBCL's FORMAT, one call per line: <control as hex> <arg>... where an
;;; argument is i<decimal integer>, r<num>/<den>, s<8 hex digits> (the bits
;;; of a single-float), d<16 hex digits> (a double-float) or n (NIL).
;;; Answers the output as hex ("-" for empty), or E when FORMAT signals an
;;; error.
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

(defun signed32 (u) (if (>= u #x80000000) (- u #x100000000) u))

(defun arg-of (tok)
  (let ((k (char tok 0)) (v (subseq tok 1)))
    (case k
      (#\i (parse-integer v))
      (#\r (let ((slash (position #\/ v)))
             (/ (parse-integer v :end slash) (parse-integer v :start (1+ slash)))))
      (#\s (sb-kernel:make-single-float (signed32 (parse-integer v :radix 16))))
      (#\d (let ((bits (parse-integer v :radix 16)))
             (sb-kernel:make-double-float (signed32 (ash bits -32)) (logand bits #xffffffff))))
      (#\c (code-char (parse-integer v)))
      (t nil))))

(defun split (line)
  (loop with start = 0
        for pos = (position #\Space line :start start)
        collect (subseq line start pos)
        while pos
        do (setf start (1+ pos))))

(progn
  (loop for line = (read-line *standard-input* nil)
        while line
        do (let* ((toks (split line))
                  (control (unhex (first toks)))
                  (args (mapcar #'arg-of (rest toks))))
             (write-line
              (handler-case (hex (apply #'format nil control args))
                (error () "E"))))))
