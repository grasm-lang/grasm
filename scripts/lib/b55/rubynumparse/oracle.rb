# The oracle for packages/rubynumparse: Ruby 2.6's String#to_i, #to_f,
# Integer() and Float(), with UTF-8 strings. A line is "ti HEX BASE",
# "ii HEX BASE" (BASE "-" for none), "tf HEX" or "ff HEX"; an integer
# answers in decimal, a float as the hex of its bits, an error as E: and
# the message.
def unhex(h)
  (h == "-" ? "" : [h].pack("H*")).force_encoding("UTF-8")
end

out = []
STDIN.each_line do |l|
  f = l.split
  next if f.empty?
  s = unhex(f[1])
  begin
    r =
      case f[0]
      when "ti" then s.to_i(f[2].to_i).to_s
      when "ii" then (f[2] == "-" ? Integer(s) : Integer(s, f[2].to_i)).to_s
      when "tf" then [s.to_f].pack("G").unpack1("H*")
      else [Float(s)].pack("G").unpack1("H*")
      end
  rescue ArgumentError => e
    r = "E:" + e.message
  end
  out << r
end
STDOUT.write(out.map { |x| x + "\n" }.join)
