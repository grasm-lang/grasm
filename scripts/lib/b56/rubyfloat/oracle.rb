# The oracle for packages/rubyfloat: Float#to_s, round (with half:),
# floor, ceil and truncate under Ruby 2.6. A float travels as the hex of
# its IEEE-754 bits; an answer is i<decimal> for an Integer, f<bits> for a
# Float (fnan for any NaN), or E:<message> for a FloatDomainError.
def num(x)
  return "i#{x}" if x.is_a?(Integer)
  x.nan? ? 'fnan' : 'f' + [x].pack('G').unpack1('H*')
end

out = []
STDIN.each_line do |l|
  f = l.split
  next if f.empty?
  f.shift
  op = f.shift
  v = [f[0]].pack('H*').unpack1('G')
  n = f[1].to_i
  out << begin
    case op
    when 'ts' then v.to_s
    when 'rd' then num(v.round(n, half: f[2].to_sym))
    when 'r0' then num(v.round)
    when 'fl' then num(v.floor(n))
    when 'ce' then num(v.ceil(n))
    else num(v.truncate(n))
    end
  rescue FloatDomainError => e
    "E:#{e.message}"
  end
end
puts out
