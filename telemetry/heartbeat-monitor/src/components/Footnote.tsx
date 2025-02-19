interface FootnoteProps {
  children: React.ReactNode;
}

const Footnote: React.FC<FootnoteProps> = ({ children }) => {
  return (
    <p className="text-xs text-gray-400 italic mb-1">
      {children}
    </p>
  );
};

export default Footnote;