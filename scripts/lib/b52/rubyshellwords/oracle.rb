# The oracle for packages/rubyshellwords: Ruby 2.6's Shellwords and
# String#inspect. Text is hex UTF-8, '-' when empty; lists are joined by ','.
require "shellwords"
Encoding.default_external = Encoding::UTF_8

def text(h)
  h == "-" ? "" : [h].pack("H*").force_encoding("UTF-8")
end

def hx(s)
  s.empty? ? "-" : s.unpack1("H*")
end

out = []
STDIN.read.split("\n").each do |l|
  next if l.empty?
  f = l.split(" ")
  begin
    out << case f[0]
           when "sp" then (w = Shellwords.split(text(f[1]))).empty? ? "~" : w.map { |x| hx(x) }.join(",")
           when "es" then hx(Shellwords.escape(text(f[1])))
           when "jo" then hx(Shellwords.join(f[1] == "~" ? [] : f[1].split(",").map { |x| text(x) }))
           else hx(text(f[1]).inspect)
           end
  rescue ArgumentError => e
    out << "E" + hx(e.message)
  end
end
print out.join("\n") + "\n"
